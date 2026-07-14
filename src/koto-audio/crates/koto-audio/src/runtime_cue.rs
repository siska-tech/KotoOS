//! Bounded Native KotoAudio KMML compiler and pointer-free runtime cue image.
//!
//! The device uses this module to compile text read from removable storage into
//! a compact image suitable for copy-based PSRAM.  The image contains no Rust
//! references or platform pointers and can therefore be decoded into caller-
//! owned SRAM immediately before playback.

use crate::{
    MixerVolume, SequenceEvent, SequencePitch, BUILTIN_INSTRUMENT_BASS_DRUM,
    BUILTIN_INSTRUMENT_CLAP, BUILTIN_INSTRUMENT_CLOSED_HI_HAT, BUILTIN_INSTRUMENT_CRASH_CYMBAL,
    BUILTIN_INSTRUMENT_OPEN_HI_HAT, BUILTIN_INSTRUMENT_SAW, BUILTIN_INSTRUMENT_SAW_FAST,
    BUILTIN_INSTRUMENT_SNARE_DRUM_1, BUILTIN_INSTRUMENT_SNARE_DRUM_2, BUILTIN_INSTRUMENT_SQUARE,
    BUILTIN_INSTRUMENT_SQUARE_FAST, BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH,
    BUILTIN_INSTRUMENT_SYNTH_TOM_LOW, BUILTIN_INSTRUMENT_SYNTH_TOM_MID,
    BUILTIN_INSTRUMENT_TRIANGLE, BUILTIN_INSTRUMENT_TRIANGLE_FAST, MAX_SEQUENCE_VOICES,
    SEQUENCE_REPEAT_INFINITE,
};

/// Magic at the start of a serialized runtime cue image.
pub const RUNTIME_CUE_MAGIC: [u8; 4] = *b"KAQ1";
/// Current pointer-free runtime cue image version.
pub const RUNTIME_CUE_VERSION: u8 = 1;
/// Maximum serialized bytes for a cue with `N` events per voice.
pub const fn runtime_cue_max_encoded_len<const N: usize>() -> usize {
    12 + MAX_SEQUENCE_VOICES * (4 + N * 8)
}

/// A failure while parsing Native KotoAudio KMML or encoding its runtime image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCueError {
    /// Input was not UTF-8/ASCII KMML supported by the device parser.
    InvalidText,
    /// A command was malformed or outside its supported range.
    InvalidCommand,
    /// A note duration cannot be represented by the runtime tick grid.
    InvalidDuration,
    /// A note octave falls outside MIDI's range.
    InvalidOctave,
    /// An instrument id or drum alias is not a Native KotoAudio builtin.
    UnknownInstrument,
    /// The score contains no playable events.
    Empty,
    /// The score contains more voices than KotoAudio supports.
    TooManyTracks,
    /// One voice exceeded the caller-selected bounded event capacity.
    TooManyEvents,
    /// Tracks selected incompatible tempos.
    TempoMismatch,
    /// The score contains a malformed or nested loop.
    InvalidLoop,
    /// The serialized destination/source is too small or malformed.
    InvalidImage,
}

/// One caller-owned voice in a compiled runtime cue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCueTrack<const N: usize> {
    /// Fixed event storage. Only `len` entries are meaningful.
    pub events: [SequenceEvent; N],
    /// Number of meaningful events, including the terminal `End`.
    pub len: usize,
    /// Voice-local gain.
    pub gain: MixerVolume,
}

impl<const N: usize> RuntimeCueTrack<N> {
    const EMPTY: Self = Self {
        events: [SequenceEvent::End; N],
        len: 0,
        gain: MixerVolume::UNITY,
    };

    fn push(&mut self, event: SequenceEvent) -> Result<(), RuntimeCueError> {
        let Some(slot) = self.events.get_mut(self.len) else {
            return Err(RuntimeCueError::TooManyEvents);
        };
        *slot = event;
        self.len += 1;
        Ok(())
    }
}

/// Owned, bounded cue compiled from Native KotoAudio KMML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCue<const N: usize> {
    /// Fixed voice storage. Only `track_count` entries are meaningful.
    pub tracks: [RuntimeCueTrack<N>; MAX_SEQUENCE_VOICES],
    /// Number of voices in the score.
    pub track_count: usize,
    /// Fixed sequence tick rate shared by all voices.
    pub tick_rate_hz: u16,
}

impl<const N: usize> RuntimeCue<N> {
    /// Creates an empty cue suitable as a static or stack buffer.
    pub const fn empty() -> Self {
        Self {
            tracks: [RuntimeCueTrack::EMPTY; MAX_SEQUENCE_VOICES],
            track_count: 0,
            tick_rate_hz: 0,
        }
    }

    /// Resets only the cue metadata in place.
    ///
    /// Event slots are overwritten as tracks are parsed/decoded, so clearing
    /// the whole backing array is unnecessary. More importantly, assigning
    /// `Self::empty()` here would materialize a multi-kilobyte temporary on an
    /// embedded caller's stack even when `self` itself lives in static SRAM.
    fn reset_in_place(&mut self) {
        self.track_count = 0;
        self.tick_rate_hz = 0;
        for track in &mut self.tracks {
            track.len = 0;
            track.gain = MixerVolume::UNITY;
        }
    }

    /// Compiles Native KotoAudio KMML into this bounded owned representation.
    pub fn compile_kmml(source: &str) -> Result<Self, RuntimeCueError> {
        let mut cue = Self::empty();
        cue.compile_kmml_into(source)?;
        Ok(cue)
    }

    /// Compiles directly into this existing buffer without a large temporary.
    pub fn compile_kmml_into(&mut self, source: &str) -> Result<(), RuntimeCueError> {
        self.reset_in_place();
        Parser::<N>::new(source.as_bytes(), self).parse()
    }

    /// Returns the exact byte count needed by [`Self::encode`].
    pub fn encoded_len(&self) -> usize {
        12 + self.tracks[..self.track_count]
            .iter()
            .map(|track| 4 + track.len * 8)
            .sum::<usize>()
    }

    /// Encodes this cue into a pointer-free little-endian PSRAM image.
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, RuntimeCueError> {
        let len = self.encoded_len();
        if self.track_count == 0 || self.track_count > MAX_SEQUENCE_VOICES || out.len() < len {
            return Err(RuntimeCueError::InvalidImage);
        }
        out[..4].copy_from_slice(&RUNTIME_CUE_MAGIC);
        out[4] = RUNTIME_CUE_VERSION;
        out[5] = self.track_count as u8;
        put_u16(out, 6, self.tick_rate_hz)?;
        put_u32(out, 8, len as u32)?;
        let mut cursor = 12;
        for track in &self.tracks[..self.track_count] {
            put_u16(out, cursor, track.gain.get())?;
            put_u16(out, cursor + 2, track.len as u16)?;
            cursor += 4;
            for event in &track.events[..track.len] {
                encode_event(*event, &mut out[cursor..cursor + 8]);
                cursor += 8;
            }
        }
        Ok(cursor)
    }

    /// Decodes a pointer-free PSRAM image into caller-owned SRAM.
    pub fn decode(bytes: &[u8]) -> Result<Self, RuntimeCueError> {
        let mut cue = Self::empty();
        cue.decode_into(bytes)?;
        Ok(cue)
    }

    /// Decodes a pointer-free PSRAM image directly into this existing buffer.
    /// This avoids materializing a large temporary on a small embedded stack.
    pub fn decode_into(&mut self, bytes: &[u8]) -> Result<(), RuntimeCueError> {
        if bytes.len() < 12 || bytes[..4] != RUNTIME_CUE_MAGIC || bytes[4] != RUNTIME_CUE_VERSION {
            return Err(RuntimeCueError::InvalidImage);
        }
        let track_count = usize::from(bytes[5]);
        let tick_rate_hz = get_u16(bytes, 6)?;
        let image_len = get_u32(bytes, 8)? as usize;
        if track_count == 0
            || track_count > MAX_SEQUENCE_VOICES
            || tick_rate_hz == 0
            || image_len != bytes.len()
        {
            return Err(RuntimeCueError::InvalidImage);
        }
        self.reset_in_place();
        self.track_count = track_count;
        self.tick_rate_hz = tick_rate_hz;
        let mut cursor = 12;
        for track in &mut self.tracks[..track_count] {
            track.gain = MixerVolume::new(get_u16(bytes, cursor)?);
            let event_count = usize::from(get_u16(bytes, cursor + 2)?);
            cursor += 4;
            if event_count == 0 || event_count > N || cursor + event_count * 8 > bytes.len() {
                return Err(RuntimeCueError::InvalidImage);
            }
            for _ in 0..event_count {
                track.push(decode_event(&bytes[cursor..cursor + 8])?)?;
                cursor += 8;
            }
            if track.events[track.len - 1] != SequenceEvent::End {
                return Err(RuntimeCueError::InvalidImage);
            }
        }
        if cursor != bytes.len() {
            return Err(RuntimeCueError::InvalidImage);
        }
        Ok(())
    }
}

struct Parser<'a, 'out, const N: usize> {
    source: &'a [u8],
    index: usize,
    cue: &'out mut RuntimeCue<N>,
    track: usize,
    track_started: bool,
    bpm: Option<u16>,
    current_bpm: u16,
    default_length: u16,
    octave: i8,
    volume: u8,
    instrument: u8,
    loop_open: bool,
    saw_event: bool,
}

impl<'a, 'out, const N: usize> Parser<'a, 'out, N> {
    fn new(source: &'a [u8], cue: &'out mut RuntimeCue<N>) -> Self {
        Self {
            source,
            index: 0,
            cue,
            track: 0,
            track_started: false,
            bpm: None,
            current_bpm: 120,
            default_length: 4,
            octave: 4,
            volume: 127,
            instrument: BUILTIN_INSTRUMENT_SQUARE_FAST,
            loop_open: false,
            saw_event: false,
        }
    }

    fn parse(mut self) -> Result<(), RuntimeCueError> {
        while let Some(ch) = self.peek() {
            match ch {
                b' ' | b'\t' | b'\r' | b'\n' => self.index += 1,
                b';' => self.skip_line(),
                b'#' if self.line_directive(b"#TRACK") => self.start_track()?,
                b'#' => self.skip_line(),
                b'/' if self.peek_next() == Some(b'/') => self.skip_line(),
                b'T' | b't' => {
                    self.index += 1;
                    self.current_bpm = self.number_u16()?;
                    if self.current_bpm == 0 {
                        return Err(RuntimeCueError::InvalidCommand);
                    }
                }
                b'L' | b'l' => {
                    self.index += 1;
                    let value = self.number_u16()?;
                    duration_ticks(value, false)?;
                    self.default_length = value;
                }
                b'O' | b'o' => {
                    self.index += 1;
                    let value = self.number_u16()?;
                    if value > 9 {
                        return Err(RuntimeCueError::InvalidOctave);
                    }
                    self.octave = value as i8;
                }
                b'<' => {
                    self.index += 1;
                    self.octave = self
                        .octave
                        .checked_sub(1)
                        .ok_or(RuntimeCueError::InvalidOctave)?;
                    if self.octave < 0 {
                        return Err(RuntimeCueError::InvalidOctave);
                    }
                }
                b'>' => {
                    self.index += 1;
                    self.octave = self
                        .octave
                        .checked_add(1)
                        .ok_or(RuntimeCueError::InvalidOctave)?;
                    if self.octave > 9 {
                        return Err(RuntimeCueError::InvalidOctave);
                    }
                }
                b'@' => {
                    self.index += 1;
                    let id = self.number_u16()?;
                    if id > u8::MAX as u16 || !valid_instrument(id as u8) {
                        return Err(RuntimeCueError::UnknownInstrument);
                    }
                    self.instrument = id as u8;
                }
                b'V' | b'v' => {
                    self.index += 1;
                    let value = self.number_u16()?;
                    if value > 127 {
                        return Err(RuntimeCueError::InvalidCommand);
                    }
                    self.volume = value as u8;
                }
                b'[' => {
                    self.ensure_track()?;
                    self.index += 1;
                    if self.loop_open {
                        return Err(RuntimeCueError::InvalidLoop);
                    }
                    self.loop_open = true;
                    self.push(SequenceEvent::LoopStart)?;
                }
                b']' => {
                    self.index += 1;
                    if !self.loop_open {
                        return Err(RuntimeCueError::InvalidLoop);
                    }
                    self.loop_open = false;
                    let repeat = match self.optional_number()? {
                        Some(0) => SEQUENCE_REPEAT_INFINITE,
                        Some(1) | None => 0,
                        Some(value) if value <= u8::MAX as u16 + 1 => (value - 1) as u8,
                        _ => return Err(RuntimeCueError::InvalidCommand),
                    };
                    self.push(SequenceEvent::LoopEnd {
                        repeat_count: repeat,
                    })?;
                }
                b'!' => self.drum()?,
                b'a'..=b'g' | b'A'..=b'G' => self.note()?,
                b'r' | b'R' => self.rest()?,
                _ => return Err(RuntimeCueError::InvalidText),
            }
        }
        self.finish_track()?;
        if self.cue.track_count == 0 {
            return Err(RuntimeCueError::Empty);
        }
        let bpm = self.bpm.ok_or(RuntimeCueError::Empty)?;
        self.cue.tick_rate_hz = (((u32::from(bpm) * 4) + 30) / 60) as u16;
        Ok(())
    }

    fn start_track(&mut self) -> Result<(), RuntimeCueError> {
        self.finish_track()?;
        if self.cue.track_count >= MAX_SEQUENCE_VOICES {
            return Err(RuntimeCueError::TooManyTracks);
        }
        self.track = self.cue.track_count;
        self.track_started = true;
        self.current_bpm = 120;
        self.default_length = 4;
        self.octave = 4;
        self.volume = 127;
        self.instrument = BUILTIN_INSTRUMENT_SQUARE_FAST;
        self.loop_open = false;
        self.saw_event = false;
        self.skip_line();
        Ok(())
    }

    fn ensure_track(&mut self) -> Result<(), RuntimeCueError> {
        if !self.track_started {
            if self.cue.track_count >= MAX_SEQUENCE_VOICES {
                return Err(RuntimeCueError::TooManyTracks);
            }
            self.track = self.cue.track_count;
            self.track_started = true;
        }
        Ok(())
    }

    fn finish_track(&mut self) -> Result<(), RuntimeCueError> {
        if !self.track_started {
            return Ok(());
        }
        if self.loop_open {
            return Err(RuntimeCueError::InvalidLoop);
        }
        if !self.saw_event {
            return Err(RuntimeCueError::Empty);
        }
        self.cue.tracks[self.track].push(SequenceEvent::End)?;
        self.cue.track_count += 1;
        match self.bpm {
            Some(expected) if expected != self.current_bpm => {
                return Err(RuntimeCueError::TempoMismatch)
            }
            None => self.bpm = Some(self.current_bpm),
            _ => {}
        }
        self.track_started = false;
        Ok(())
    }

    fn note(&mut self) -> Result<(), RuntimeCueError> {
        self.ensure_track()?;
        let note = self.next().ok_or(RuntimeCueError::InvalidText)?;
        let accidental = match self.peek() {
            Some(b'+') | Some(b'#') => {
                self.index += 1;
                1
            }
            Some(b'-') => {
                self.index += 1;
                -1
            }
            _ => 0,
        };
        let (length, dotted) = self.length()?;
        let duration_ticks = duration_ticks(length, dotted)?;
        let base = match note.to_ascii_lowercase() {
            b'c' => 0,
            b'd' => 2,
            b'e' => 4,
            b'f' => 5,
            b'g' => 7,
            b'a' => 9,
            b'b' => 11,
            _ => return Err(RuntimeCueError::InvalidText),
        };
        let midi = i16::from(self.octave + 1) * 12 + base + accidental;
        let midi = u8::try_from(midi).map_err(|_| RuntimeCueError::InvalidOctave)?;
        self.push(SequenceEvent::Note {
            pitch: SequencePitch::from_midi_note(midi).frequency_hz,
            duration_ticks,
            volume: self.volume,
            instrument_id: self.instrument,
        })
    }

    fn rest(&mut self) -> Result<(), RuntimeCueError> {
        self.ensure_track()?;
        self.index += 1;
        let (length, dotted) = self.length()?;
        self.push(SequenceEvent::Rest {
            duration_ticks: duration_ticks(length, dotted)?,
        })
    }

    fn drum(&mut self) -> Result<(), RuntimeCueError> {
        self.ensure_track()?;
        self.index += 1;
        let aliases: &[(&[u8], u8)] = &[
            (b"bd", BUILTIN_INSTRUMENT_BASS_DRUM),
            (b"sd", BUILTIN_INSTRUMENT_SNARE_DRUM_1),
            (b"s2", BUILTIN_INSTRUMENT_SNARE_DRUM_2),
            (b"hh", BUILTIN_INSTRUMENT_CLOSED_HI_HAT),
            (b"oh", BUILTIN_INSTRUMENT_OPEN_HI_HAT),
            (b"cy", BUILTIN_INSTRUMENT_CRASH_CYMBAL),
            (b"th", BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH),
            (b"tm", BUILTIN_INSTRUMENT_SYNTH_TOM_MID),
            (b"tl", BUILTIN_INSTRUMENT_SYNTH_TOM_LOW),
            (b"cl", BUILTIN_INSTRUMENT_CLAP),
        ];
        let Some((alias, instrument)) = aliases.iter().find(|(alias, _)| self.starts_with(alias))
        else {
            return Err(RuntimeCueError::UnknownInstrument);
        };
        self.index += alias.len();
        let instrument = *instrument;
        let (length, dotted) = self.length()?;
        self.push(SequenceEvent::Note {
            pitch: SequencePitch::from_midi_note(60).frequency_hz,
            duration_ticks: duration_ticks(length, dotted)?,
            volume: self.volume,
            instrument_id: instrument,
        })
    }

    fn push(&mut self, event: SequenceEvent) -> Result<(), RuntimeCueError> {
        self.cue.tracks[self.track].push(event)?;
        self.saw_event = true;
        Ok(())
    }

    fn length(&mut self) -> Result<(u16, bool), RuntimeCueError> {
        let length = self.optional_number()?.unwrap_or(self.default_length);
        let dotted = if self.peek() == Some(b'.') {
            self.index += 1;
            if self.peek() == Some(b'.') {
                return Err(RuntimeCueError::InvalidDuration);
            }
            true
        } else {
            false
        };
        Ok((length, dotted))
    }

    fn number_u16(&mut self) -> Result<u16, RuntimeCueError> {
        self.optional_number()?
            .ok_or(RuntimeCueError::InvalidCommand)
    }

    fn optional_number(&mut self) -> Result<Option<u16>, RuntimeCueError> {
        let start = self.index;
        let mut value = 0u32;
        while let Some(ch @ b'0'..=b'9') = self.peek() {
            self.index += 1;
            value = value
                .checked_mul(10)
                .and_then(|n| n.checked_add(u32::from(ch - b'0')))
                .ok_or(RuntimeCueError::InvalidCommand)?;
            if value > u16::MAX as u32 {
                return Err(RuntimeCueError::InvalidCommand);
            }
        }
        Ok((self.index != start).then_some(value as u16))
    }

    fn line_directive(&self, directive: &[u8]) -> bool {
        self.source
            .get(self.index..self.index + directive.len())
            .is_some_and(|actual| actual.eq_ignore_ascii_case(directive))
    }
    fn starts_with(&self, text: &[u8]) -> bool {
        self.source
            .get(self.index..self.index + text.len())
            .is_some_and(|actual| actual.eq_ignore_ascii_case(text))
    }
    fn skip_line(&mut self) {
        while let Some(ch) = self.peek() {
            self.index += 1;
            if ch == b'\n' {
                break;
            }
        }
    }
    fn peek(&self) -> Option<u8> {
        self.source.get(self.index).copied()
    }
    fn peek_next(&self) -> Option<u8> {
        self.source.get(self.index + 1).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let ch = self.peek()?;
        self.index += 1;
        Some(ch)
    }
}

fn duration_ticks(denominator: u16, dotted: bool) -> Result<u16, RuntimeCueError> {
    if denominator == 0 || 16 % denominator != 0 {
        return Err(RuntimeCueError::InvalidDuration);
    }
    let ticks = 16 / denominator;
    if dotted {
        let value = u32::from(ticks) * 3;
        if value % 2 != 0 {
            return Err(RuntimeCueError::InvalidDuration);
        }
        Ok((value / 2) as u16)
    } else {
        Ok(ticks)
    }
}

fn valid_instrument(id: u8) -> bool {
    matches!(
        id,
        BUILTIN_INSTRUMENT_SQUARE_FAST
            | BUILTIN_INSTRUMENT_SAW_FAST
            | BUILTIN_INSTRUMENT_TRIANGLE_FAST
            | BUILTIN_INSTRUMENT_SQUARE
            | BUILTIN_INSTRUMENT_SAW
            | BUILTIN_INSTRUMENT_TRIANGLE
            | BUILTIN_INSTRUMENT_BASS_DRUM
            | BUILTIN_INSTRUMENT_SNARE_DRUM_2
            | BUILTIN_INSTRUMENT_SNARE_DRUM_1
            | BUILTIN_INSTRUMENT_OPEN_HI_HAT
            | BUILTIN_INSTRUMENT_CLOSED_HI_HAT
            | BUILTIN_INSTRUMENT_CRASH_CYMBAL
            | BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH
            | BUILTIN_INSTRUMENT_SYNTH_TOM_MID
            | BUILTIN_INSTRUMENT_SYNTH_TOM_LOW
            | BUILTIN_INSTRUMENT_CLAP
    )
}

fn encode_event(event: SequenceEvent, out: &mut [u8]) {
    out.fill(0);
    match event {
        SequenceEvent::Note {
            pitch,
            duration_ticks,
            volume,
            instrument_id,
        } => {
            out[0] = 1;
            out[1] = instrument_id;
            out[2] = volume;
            out[4..6].copy_from_slice(&pitch.to_le_bytes());
            out[6..8].copy_from_slice(&duration_ticks.to_le_bytes());
        }
        SequenceEvent::Rest { duration_ticks } => {
            out[0] = 2;
            out[6..8].copy_from_slice(&duration_ticks.to_le_bytes());
        }
        SequenceEvent::LoopStart => out[0] = 3,
        SequenceEvent::LoopEnd { repeat_count } => {
            out[0] = 4;
            out[1] = repeat_count;
        }
        SequenceEvent::End => out[0] = 5,
    }
}

fn decode_event(bytes: &[u8]) -> Result<SequenceEvent, RuntimeCueError> {
    if bytes.len() < 8 {
        return Err(RuntimeCueError::InvalidImage);
    }
    Ok(match bytes[0] {
        1 if valid_instrument(bytes[1]) => SequenceEvent::Note {
            pitch: u16::from_le_bytes([bytes[4], bytes[5]]),
            duration_ticks: u16::from_le_bytes([bytes[6], bytes[7]]),
            volume: bytes[2],
            instrument_id: bytes[1],
        },
        2 => SequenceEvent::Rest {
            duration_ticks: u16::from_le_bytes([bytes[6], bytes[7]]),
        },
        3 => SequenceEvent::LoopStart,
        4 => SequenceEvent::LoopEnd {
            repeat_count: bytes[1],
        },
        5 => SequenceEvent::End,
        _ => return Err(RuntimeCueError::InvalidImage),
    })
}

fn put_u16(out: &mut [u8], at: usize, value: u16) -> Result<(), RuntimeCueError> {
    let dst = out
        .get_mut(at..at + 2)
        .ok_or(RuntimeCueError::InvalidImage)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
fn put_u32(out: &mut [u8], at: usize, value: u32) -> Result<(), RuntimeCueError> {
    let dst = out
        .get_mut(at..at + 4)
        .ok_or(RuntimeCueError::InvalidImage)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
fn get_u16(bytes: &[u8], at: usize) -> Result<u16, RuntimeCueError> {
    let src = bytes.get(at..at + 2).ok_or(RuntimeCueError::InvalidImage)?;
    Ok(u16::from_le_bytes([src[0], src[1]]))
}
fn get_u32(bytes: &[u8], at: usize) -> Result<u32, RuntimeCueError> {
    let src = bytes.get(at..at + 4).ok_or(RuntimeCueError::InvalidImage)?;
    Ok(u32::from_le_bytes([src[0], src[1], src[2], src[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    #[test]
    fn native_multitrack_drums_round_trip_pointer_free_image() {
        let text = "; native\n#TRACK lead\n@3 T120 V100 O4 L8 [c e g]0\n#TRACK drums\nT120 V90 L16 [!bd r !hh !sd]0\n";
        let cue = RuntimeCue::<32>::compile_kmml(text).unwrap();
        assert_eq!(cue.track_count, 2);
        assert_eq!(cue.tick_rate_hz, 8);
        let mut image = [0u8; runtime_cue_max_encoded_len::<32>()];
        let len = cue.encode(&mut image).unwrap();
        assert_eq!(RuntimeCue::<32>::decode(&image[..len]).unwrap(), cue);
    }

    #[test]
    fn capacity_and_tempo_mismatch_are_rejected() {
        assert_eq!(
            RuntimeCue::<2>::compile_kmml("c d"),
            Err(RuntimeCueError::TooManyEvents)
        );
        assert_eq!(
            RuntimeCue::<8>::compile_kmml("#TRACK a\nT120 c\n#TRACK b\nT121 c"),
            Err(RuntimeCueError::TempoMismatch)
        );
    }

    #[test]
    fn into_operations_reuse_the_existing_storage_without_stale_metadata() {
        let mut cue = RuntimeCue::<32>::empty();
        cue.compile_kmml_into("#TRACK a\nc d e\n#TRACK b\n!bd !sd")
            .unwrap();
        assert_eq!(cue.track_count, 2);

        cue.compile_kmml_into("#TRACK only\n!hh").unwrap();
        assert_eq!(cue.track_count, 1);
        assert_eq!(cue.tracks[0].len, 2);
        assert_eq!(cue.tracks[1].len, 0);

        let mut image = [0u8; runtime_cue_max_encoded_len::<32>()];
        let len = cue.encode(&mut image).unwrap();
        cue.compile_kmml_into("c d e f g a b").unwrap();
        cue.decode_into(&image[..len]).unwrap();
        assert_eq!(cue.track_count, 1);
        assert_eq!(cue.tracks[0].len, 2);
    }

    #[test]
    fn every_shipped_sd_kmml_fits_the_device_runtime_bounds() {
        use std::{fs, path::PathBuf};

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../apps");
        let mut count = 0usize;
        for app in fs::read_dir(root).unwrap() {
            let audio = app.unwrap().path().join("audio");
            let Ok(files) = fs::read_dir(audio) else {
                continue;
            };
            for file in files {
                let path = file.unwrap().path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("kmml") {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                let cue = RuntimeCue::<272>::compile_kmml(&text)
                    .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
                let is_bgm = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.starts_with("bgm"));
                if is_bgm {
                    for track in &cue.tracks[..cue.track_count] {
                        assert!(
                            track.events[..track.len].iter().any(|event| matches!(
                                event,
                                SequenceEvent::LoopEnd {
                                    repeat_count: SEQUENCE_REPEAT_INFINITE
                                }
                            )),
                            "{} has a non-looping BGM track",
                            path.display()
                        );
                    }
                } else {
                    RuntimeCue::<32>::compile_kmml(&text)
                        .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
                }
                count += 1;
            }
        }
        assert_eq!(count, 48);
    }

    #[test]
    fn every_shipped_bgm_runtime_image_renders_non_silence() {
        use crate::{DecodeResult, RuntimeCuePlayer};
        use std::{fs, path::PathBuf};

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../apps");
        let mut bgm_count = 0usize;
        for app in fs::read_dir(root).unwrap() {
            let audio = app.unwrap().path().join("audio");
            let Ok(files) = fs::read_dir(audio) else {
                continue;
            };
            for file in files {
                let path = file.unwrap().path();
                let is_bgm = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.starts_with("bgm"));
                if path.extension().and_then(|ext| ext.to_str()) != Some("kmml") || !is_bgm {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                let cue = RuntimeCue::<272>::compile_kmml(&text).unwrap();
                let mut image = [0u8; runtime_cue_max_encoded_len::<272>()];
                let len = cue.encode(&mut image).unwrap();
                let mut player = RuntimeCuePlayer::<272>::new(16_000);
                player.play_image(&image[..len]).unwrap();
                let audible = (0..16_000)
                    .filter(|_| matches!(player.next_sample(), DecodeResult::Sample(sample) if sample != 0))
                    .count();
                assert!(
                    audible > 100,
                    "{} rendered effectively silent",
                    path.display()
                );
                assert!(
                    player.is_playing(),
                    "{} stopped within one second",
                    path.display()
                );
                bgm_count += 1;
            }
        }
        assert_eq!(bgm_count, 9);
    }
}
