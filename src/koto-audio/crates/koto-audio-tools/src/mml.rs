//! Experimental host-side KotoMML subset parser bridge.
//!
//! This module intentionally lives in `koto-audio-tools`, uses `std`, and emits
//! the existing compact sequence table shape. Runtime playback remains a
//! compact sequence consumer; it does not parse MML, load `.kmml`, allocate, or
//! perform WAV/resampling work.

use koto_audio::{
    validate_compact_sequence, CompactEvent, CompactInstrument, CompactSequence,
    CompactSequenceError, CompactTempo, CompactTrack, MixerVolume, SequencePitch,
    BUILTIN_INSTRUMENT_BASS_DRUM, BUILTIN_INSTRUMENT_CLAP, BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
    BUILTIN_INSTRUMENT_CRASH_CYMBAL, BUILTIN_INSTRUMENT_OPEN_HI_HAT, BUILTIN_INSTRUMENT_SAW,
    BUILTIN_INSTRUMENT_SAW_FAST, BUILTIN_INSTRUMENT_SNARE_DRUM_1, BUILTIN_INSTRUMENT_SNARE_DRUM_2,
    BUILTIN_INSTRUMENT_SQUARE, BUILTIN_INSTRUMENT_SQUARE_FAST, BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH,
    BUILTIN_INSTRUMENT_SYNTH_TOM_LOW, BUILTIN_INSTRUMENT_SYNTH_TOM_MID,
    BUILTIN_INSTRUMENT_TRIANGLE, BUILTIN_INSTRUMENT_TRIANGLE_FAST, MAX_SEQUENCE_VOICES,
    SEQUENCE_REPEAT_INFINITE,
};

use crate::{CompactSequenceTable, CompactSequenceTableTrack};

/// Parser options for the experimental M13 KotoMML subset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmlParseOptions {
    /// Sequence ticks per quarter-note beat.
    pub ticks_per_beat: u16,
    /// Track-local fixed-point gain, where 256 is unity.
    pub track_gain: MixerVolume,
    /// Built-in instrument selected before the first explicit `@` command.
    pub default_builtin_instrument_id: u8,
    /// Note-local volume selected before the first explicit `V` command.
    pub default_volume: u8,
    /// Default note length denominator selected before the first `L` command.
    pub default_length: u16,
    /// Default octave selected before the first `O` command.
    pub default_octave: i8,
    /// Default tempo selected before the first `T` command.
    pub default_bpm: u16,
}

impl Default for MmlParseOptions {
    fn default() -> Self {
        Self {
            ticks_per_beat: 4,
            track_gain: MixerVolume::UNITY,
            default_builtin_instrument_id: BUILTIN_INSTRUMENT_SQUARE_FAST,
            default_volume: 127,
            default_length: 4,
            default_octave: 4,
            default_bpm: 120,
        }
    }
}

/// Detailed parser failure for the experimental M13 subset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MmlParseError {
    /// Parser options were outside the supported subset.
    InvalidOptions,
    /// Input did not contain any note, rest, or loop events.
    EmptyTrack,
    /// The input requested more tracks than the runtime sequence voice cap.
    TooManyTracks,
    /// The input referenced a built-in id that is unknown or reserved.
    UnknownInstrumentId(u8),
    /// The input referenced an unknown experimental drum alias.
    UnknownDrumAlias(String),
    /// Multiple tracks used different tempos. The M13 subset requires one tempo.
    TempoMismatch { expected: u16, actual: u16 },
    /// A numeric command value was missing or outside the accepted range.
    InvalidNumber { command: char },
    /// A note/rest length converted to zero ticks or did not divide a whole note.
    InvalidDuration,
    /// A note octave converted outside the MIDI note range.
    InvalidOctave,
    /// A loop was opened while another loop was already active.
    NestedLoop,
    /// A loop end appeared without a matching loop start.
    LoopEndWithoutStart,
    /// A loop start did not have a matching loop end.
    UnclosedLoop,
    /// The parser encountered a character outside the M13 subset.
    UnexpectedChar(char),
    /// Runtime compact sequence validation rejected the generated table.
    RuntimeValidation(CompactSequenceError),
}

/// Parses the experimental M13 KotoMML subset into a compact sequence table.
///
/// Supported commands are `#TRACK name` separators, line comments, `T`, `L`,
/// `O`, `<`, `>`, natural notes `cdefgab`, sharps like `c+`/`c#`, single-dot
/// note/rest lengths like `c8.`/`r4.`,
/// rest `r`, built-in instrument `@0` through `@16` with reserved ids rejected,
/// experimental drum aliases like `!bd` and `!hh16`, volume `V0` through
/// `V127`, and one-level loops like `[cdef]2`.
pub fn parse_mml_to_compact_sequence_table(
    source: &str,
) -> Result<CompactSequenceTable, MmlParseError> {
    parse_mml_to_compact_sequence_table_with_options(source, MmlParseOptions::default())
}

/// Parses with explicit options for tests and future tool frontends.
pub fn parse_mml_to_compact_sequence_table_with_options(
    source: &str,
    options: MmlParseOptions,
) -> Result<CompactSequenceTable, MmlParseError> {
    if options.ticks_per_beat == 0
        || options.default_bpm == 0
        || options.default_length == 0
        || !valid_builtin_instrument_id(options.default_builtin_instrument_id)
        || options.default_volume > 127
    {
        return Err(MmlParseError::InvalidOptions);
    }

    let table = parse_tracks(source, options)?;
    let runtime_tracks: Vec<CompactTrack<'_>> = table
        .tracks
        .iter()
        .map(|track| CompactTrack::new(&track.events, track.gain, track.initial_instrument_id))
        .collect();
    validate_compact_sequence(CompactSequence::new(
        &table.instruments,
        &runtime_tracks,
        table.tempo,
    ))
    .map_err(MmlParseError::RuntimeValidation)?;
    Ok(table)
}

fn parse_tracks(
    source: &str,
    options: MmlParseOptions,
) -> Result<CompactSequenceTable, MmlParseError> {
    let sources = split_track_sources(source)?;
    if sources.len() > MAX_SEQUENCE_VOICES {
        return Err(MmlParseError::TooManyTracks);
    }

    let mut instruments = Vec::new();
    let mut tracks = Vec::with_capacity(sources.len());
    let mut sequence_bpm = None;

    for track_source in sources {
        let mut parser = Parser::new(&track_source, options.clone(), &mut instruments);
        let parsed = parser.parse_track()?;
        match sequence_bpm {
            Some(expected) if expected != parsed.bpm => {
                return Err(MmlParseError::TempoMismatch {
                    expected,
                    actual: parsed.bpm,
                });
            }
            Some(_) => {}
            None => sequence_bpm = Some(parsed.bpm),
        }
        tracks.push(parsed.track);
    }

    let bpm = sequence_bpm.ok_or(MmlParseError::EmptyTrack)?;
    Ok(CompactSequenceTable {
        instruments,
        tracks,
        tempo: CompactTempo {
            tick_rate_hz: tempo_tick_rate_hz(bpm, options.ticks_per_beat)?,
            bpm,
            ticks_per_beat: options.ticks_per_beat,
        },
    })
}

fn split_track_sources(source: &str) -> Result<Vec<String>, MmlParseError> {
    let has_track_directive = source.lines().any(|line| {
        let uncommented = strip_line_comment(line);
        is_track_directive(uncommented.trim_start())
    });
    if !has_track_directive {
        return Ok(vec![source.to_string()]);
    }

    let mut tracks = Vec::new();
    let mut current = String::new();
    let mut saw_track = false;
    for line in source.lines() {
        let uncommented = strip_line_comment(line);
        let trimmed = uncommented.trim_start();
        if is_track_directive(trimmed) {
            if saw_track {
                tracks.push(core::mem::take(&mut current));
            }
            saw_track = true;
        } else if saw_track {
            current.push_str(uncommented);
            current.push('\n');
        }
    }
    if saw_track {
        tracks.push(current);
    }

    Ok(tracks)
}

fn is_track_directive(trimmed_line: &str) -> bool {
    let Some(rest) = trimmed_line.strip_prefix("#TRACK") else {
        return false;
    };
    rest.is_empty() || rest.starts_with(char::is_whitespace)
}

fn strip_line_comment(line: &str) -> &str {
    let semicolon = line.find(';');
    let slash_slash = line.find("//");
    match (semicolon, slash_slash) {
        (Some(a), Some(b)) => &line[..a.min(b)],
        (Some(index), None) | (None, Some(index)) => &line[..index],
        (None, None) => line,
    }
}

struct ParsedTrack {
    track: CompactSequenceTableTrack,
    bpm: u16,
}

struct Parser<'a> {
    chars: Vec<char>,
    index: usize,
    options: MmlParseOptions,
    bpm: u16,
    default_length: u16,
    octave: i8,
    volume: u8,
    current_instrument: u8,
    instruments: &'a mut Vec<CompactInstrument>,
    current_instrument_index: u8,
    events: Vec<CompactEvent>,
    loop_open: bool,
    saw_event: bool,
}

impl<'a> Parser<'a> {
    fn new(
        source: &str,
        options: MmlParseOptions,
        instruments: &'a mut Vec<CompactInstrument>,
    ) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            bpm: options.default_bpm,
            default_length: options.default_length,
            octave: options.default_octave,
            volume: options.default_volume,
            current_instrument: options.default_builtin_instrument_id,
            instruments,
            current_instrument_index: 0,
            events: Vec::new(),
            loop_open: false,
            saw_event: false,
            options,
        }
    }

    fn parse_track(&mut self) -> Result<ParsedTrack, MmlParseError> {
        while let Some(ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.index += 1;
                }
                ';' | '#' => self.skip_line_comment(),
                '/' if self.peek_next() == Some('/') => self.skip_line_comment(),
                'T' | 't' => {
                    self.index += 1;
                    self.bpm = self.parse_u16('T')?;
                    if self.bpm == 0 {
                        return Err(MmlParseError::InvalidNumber { command: 'T' });
                    }
                }
                'L' | 'l' => {
                    self.index += 1;
                    let length = self.parse_u16('L')?;
                    duration_ticks(
                        ParsedLength {
                            denominator: length,
                            dotted: false,
                        },
                        self.options.ticks_per_beat,
                    )?;
                    self.default_length = length;
                }
                'O' | 'o' => {
                    self.index += 1;
                    let octave = self.parse_u16('O')?;
                    self.octave = i8::try_from(octave).map_err(|_| MmlParseError::InvalidOctave)?;
                    if !(0..=9).contains(&self.octave) {
                        return Err(MmlParseError::InvalidOctave);
                    }
                }
                '<' => {
                    self.index += 1;
                    self.octave = self
                        .octave
                        .checked_sub(1)
                        .ok_or(MmlParseError::InvalidOctave)?;
                    if self.octave < 0 {
                        return Err(MmlParseError::InvalidOctave);
                    }
                }
                '>' => {
                    self.index += 1;
                    self.octave = self
                        .octave
                        .checked_add(1)
                        .ok_or(MmlParseError::InvalidOctave)?;
                    if self.octave > 9 {
                        return Err(MmlParseError::InvalidOctave);
                    }
                }
                '@' => {
                    self.index += 1;
                    let id = self.parse_u8('@')?;
                    if !valid_builtin_instrument_id(id) {
                        return Err(MmlParseError::UnknownInstrumentId(id));
                    }
                    self.current_instrument = id;
                    self.current_instrument_index = self.intern_instrument(id)?;
                }
                'V' | 'v' => {
                    self.index += 1;
                    let volume = self.parse_u8('V')?;
                    if volume > 127 {
                        return Err(MmlParseError::InvalidNumber { command: 'V' });
                    }
                    self.volume = volume;
                }
                '[' => {
                    self.index += 1;
                    if self.loop_open {
                        return Err(MmlParseError::NestedLoop);
                    }
                    self.loop_open = true;
                    self.events.push(CompactEvent::LoopStart);
                    self.saw_event = true;
                }
                ']' => {
                    self.index += 1;
                    if !self.loop_open {
                        return Err(MmlParseError::LoopEndWithoutStart);
                    }
                    self.loop_open = false;
                    let repeat_count = self.parse_loop_repeat_count()?;
                    self.events.push(CompactEvent::LoopEnd { repeat_count });
                    self.saw_event = true;
                }
                '!' => self.parse_drum_alias()?,
                'a'..='g' | 'A'..='G' => self.parse_note()?,
                'r' | 'R' => self.parse_rest()?,
                _ => return Err(MmlParseError::UnexpectedChar(ch)),
            }
        }

        if self.loop_open {
            return Err(MmlParseError::UnclosedLoop);
        }
        if !self.saw_event {
            return Err(MmlParseError::EmptyTrack);
        }

        self.events.push(CompactEvent::End);
        let track = CompactSequenceTableTrack {
            events: core::mem::take(&mut self.events),
            gain: self.options.track_gain,
            initial_instrument_id: 0,
        };
        Ok(ParsedTrack {
            track,
            bpm: self.bpm,
        })
    }

    fn parse_note(&mut self) -> Result<(), MmlParseError> {
        let note = self.next().ok_or(MmlParseError::UnexpectedChar('\0'))?;
        self.current_instrument_index = self.intern_instrument(self.current_instrument)?;
        let semitone_offset = self.parse_accidental()?;
        let length = self.parse_length_with_default(self.default_length)?;
        let duration_ticks = duration_ticks(length, self.options.ticks_per_beat)?;
        let midi_note = midi_note(note, self.octave, semitone_offset)?;
        self.events.push(CompactEvent::Note {
            pitch: SequencePitch::from_midi_note(midi_note).frequency_hz,
            duration_ticks,
            volume: self.volume,
            instrument_id: self.current_instrument_index,
        });
        self.saw_event = true;
        Ok(())
    }

    fn parse_drum_alias(&mut self) -> Result<(), MmlParseError> {
        self.index += 1;
        let alias_start = self.index;
        let Some((alias_len, builtin_id)) = self.match_drum_alias() else {
            return Err(MmlParseError::UnknownDrumAlias(
                self.read_unknown_alias(alias_start),
            ));
        };
        self.index += alias_len;

        let instrument_id = self.intern_instrument(builtin_id)?;
        let length = self.parse_length_with_default(self.default_length)?;
        let duration_ticks = duration_ticks(length, self.options.ticks_per_beat)?;
        self.events.push(CompactEvent::Note {
            pitch: SequencePitch::from_midi_note(60).frequency_hz,
            duration_ticks,
            volume: self.volume,
            instrument_id,
        });
        self.saw_event = true;
        Ok(())
    }

    fn match_drum_alias(&self) -> Option<(usize, u8)> {
        const DRUM_ALIASES: &[(&str, u8)] = &[
            ("bd", BUILTIN_INSTRUMENT_BASS_DRUM),
            ("sd", BUILTIN_INSTRUMENT_SNARE_DRUM_1),
            ("s2", BUILTIN_INSTRUMENT_SNARE_DRUM_2),
            ("hh", BUILTIN_INSTRUMENT_CLOSED_HI_HAT),
            ("oh", BUILTIN_INSTRUMENT_OPEN_HI_HAT),
            ("cy", BUILTIN_INSTRUMENT_CRASH_CYMBAL),
            ("th", BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH),
            ("tm", BUILTIN_INSTRUMENT_SYNTH_TOM_MID),
            ("tl", BUILTIN_INSTRUMENT_SYNTH_TOM_LOW),
            ("cl", BUILTIN_INSTRUMENT_CLAP),
        ];

        DRUM_ALIASES.iter().find_map(|(alias, builtin_id)| {
            if self.remaining_starts_with(alias) {
                Some((alias.chars().count(), *builtin_id))
            } else {
                None
            }
        })
    }

    fn read_unknown_alias(&mut self, alias_start: usize) -> String {
        while let Some(ch) = self.peek() {
            if !ch.is_ascii_alphanumeric() {
                break;
            }
            self.index += 1;
        }
        self.chars[alias_start..self.index].iter().collect()
    }

    fn parse_rest(&mut self) -> Result<(), MmlParseError> {
        self.index += 1;
        let length = self.parse_length_with_default(self.default_length)?;
        self.events.push(CompactEvent::Rest {
            duration_ticks: duration_ticks(length, self.options.ticks_per_beat)?,
        });
        self.saw_event = true;
        Ok(())
    }

    fn intern_instrument(&mut self, builtin_id: u8) -> Result<u8, MmlParseError> {
        if let Some(index) = self
            .instruments
            .iter()
            .position(|instrument| instrument.builtin_id == builtin_id)
        {
            return u8::try_from(index).map_err(|_| MmlParseError::TooManyTracks);
        }
        let index =
            u8::try_from(self.instruments.len()).map_err(|_| MmlParseError::TooManyTracks)?;
        self.instruments
            .push(CompactInstrument::builtin(builtin_id, 255));
        Ok(index)
    }

    fn parse_loop_repeat_count(&mut self) -> Result<u8, MmlParseError> {
        match self.parse_optional_u16()? {
            Some(0) => Ok(SEQUENCE_REPEAT_INFINITE),
            Some(1) | None => Ok(0),
            Some(count) => {
                u8::try_from(count - 1).map_err(|_| MmlParseError::InvalidNumber { command: ']' })
            }
        }
    }

    fn parse_length_with_default(
        &mut self,
        default_length: u16,
    ) -> Result<ParsedLength, MmlParseError> {
        let denominator = self.parse_optional_u16()?.unwrap_or(default_length);
        let dotted = if self.peek() == Some('.') {
            self.index += 1;
            if self.peek() == Some('.') {
                return Err(MmlParseError::InvalidDuration);
            }
            true
        } else {
            false
        };
        Ok(ParsedLength {
            denominator,
            dotted,
        })
    }

    fn parse_accidental(&mut self) -> Result<i8, MmlParseError> {
        match self.peek() {
            Some('+') | Some('#') => {
                self.index += 1;
                Ok(1)
            }
            Some('-') => {
                self.index += 1;
                Ok(-1)
            }
            _ => Ok(0),
        }
    }

    fn parse_u16(&mut self, command: char) -> Result<u16, MmlParseError> {
        self.parse_optional_u16()?
            .ok_or(MmlParseError::InvalidNumber { command })
    }

    fn parse_u8(&mut self, command: char) -> Result<u8, MmlParseError> {
        let value = self.parse_u16(command)?;
        u8::try_from(value).map_err(|_| MmlParseError::InvalidNumber { command })
    }

    fn parse_optional_u16(&mut self) -> Result<Option<u16>, MmlParseError> {
        let start = self.index;
        let mut value: u32 = 0;
        while let Some(ch) = self.peek() {
            let Some(digit) = ch.to_digit(10) else {
                break;
            };
            self.index += 1;
            value = value
                .checked_mul(10)
                .and_then(|current| current.checked_add(digit))
                .ok_or(MmlParseError::InvalidNumber { command: '?' })?;
            if value > u32::from(u16::MAX) {
                return Err(MmlParseError::InvalidNumber { command: '?' });
            }
        }
        if self.index == start {
            Ok(None)
        } else {
            Ok(Some(value as u16))
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek() {
            self.index += 1;
            if ch == '\n' {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.index + 1).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        Some(ch)
    }

    fn remaining_starts_with(&self, needle: &str) -> bool {
        self.chars[self.index..]
            .iter()
            .copied()
            .map(|ch| ch.to_ascii_lowercase())
            .zip(needle.chars())
            .all(|(actual, expected)| actual == expected)
            && self.chars.len().saturating_sub(self.index) >= needle.chars().count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedLength {
    denominator: u16,
    dotted: bool,
}

fn duration_ticks(length: ParsedLength, ticks_per_beat: u16) -> Result<u16, MmlParseError> {
    if length.denominator == 0 {
        return Err(MmlParseError::InvalidDuration);
    }
    let whole_ticks = ticks_per_beat
        .checked_mul(4)
        .ok_or(MmlParseError::InvalidDuration)?;
    if whole_ticks % length.denominator != 0 {
        return Err(MmlParseError::InvalidDuration);
    }
    let mut ticks = whole_ticks / length.denominator;
    if length.dotted {
        let dotted_ticks = u32::from(ticks)
            .checked_mul(3)
            .ok_or(MmlParseError::InvalidDuration)?;
        if dotted_ticks % 2 != 0 {
            return Err(MmlParseError::InvalidDuration);
        }
        ticks = u16::try_from(dotted_ticks / 2).map_err(|_| MmlParseError::InvalidDuration)?;
    }
    if ticks == 0 {
        return Err(MmlParseError::InvalidDuration);
    }
    Ok(ticks)
}

fn tempo_tick_rate_hz(bpm: u16, ticks_per_beat: u16) -> Result<u16, MmlParseError> {
    let ticks_per_minute = u32::from(bpm)
        .checked_mul(u32::from(ticks_per_beat))
        .ok_or(MmlParseError::InvalidNumber { command: 'T' })?;
    let rounded = (ticks_per_minute + 30) / 60;
    u16::try_from(rounded).map_err(|_| MmlParseError::InvalidNumber { command: 'T' })
}

fn midi_note(note: char, octave: i8, semitone_offset: i8) -> Result<u8, MmlParseError> {
    let semitone = match note.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return Err(MmlParseError::UnexpectedChar(note)),
    };
    let midi = i16::from(octave + 1) * 12 + semitone + i16::from(semitone_offset);
    u8::try_from(midi).map_err(|_| MmlParseError::InvalidOctave)
}

fn valid_builtin_instrument_id(id: u8) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        format_compact_sequence_table, CompactSequenceTableError, CompactSequenceTableOptions,
    };

    #[test]
    fn simple_melody_parses_to_valid_compact_sequence_table() {
        let table = parse_mml_to_compact_sequence_table("T120 L4 O4 @0 V100 c d e r").unwrap();

        assert_eq!(table.tempo.bpm, 120);
        assert_eq!(table.tempo.tick_rate_hz, 8);
        assert_eq!(
            table.instruments[0].builtin_id,
            BUILTIN_INSTRUMENT_SQUARE_FAST
        );
        assert_eq!(table.tracks.len(), 1);
        assert_eq!(table.tracks[0].events.len(), 5);
    }

    #[test]
    fn two_track_melody_and_bass_parse() {
        let table = parse_mml_to_compact_sequence_table(
            "#TRACK melody\nT120 L8 O5 @0 c d e g\n#TRACK bass\nT120 L4 O3 @2 c r g r",
        )
        .unwrap();

        assert_eq!(table.tempo.bpm, 120);
        assert_eq!(table.tracks.len(), 2);
        assert_eq!(table.tracks[0].events.len(), 5);
        assert_eq!(table.tracks[1].events.len(), 5);
        assert_eq!(
            table.instruments[0].builtin_id,
            BUILTIN_INSTRUMENT_SQUARE_FAST
        );
        assert_eq!(
            table.instruments[1].builtin_id,
            BUILTIN_INSTRUMENT_TRIANGLE_FAST
        );
    }

    #[test]
    fn three_track_melody_bass_and_drums_parse() {
        let table = parse_mml_to_compact_sequence_table(
            "#TRACK melody\nT120 L8 O5 @0 c d e g\n\
             #TRACK bass\nT120 L4 O3 @2 c r g r\n\
             #TRACK drums\nT120 L16 !bd !hh !sd !hh",
        )
        .unwrap();

        assert_eq!(table.tracks.len(), 3);
        assert_eq!(
            table
                .instruments
                .iter()
                .map(|instrument| instrument.builtin_id)
                .collect::<Vec<_>>(),
            vec![
                BUILTIN_INSTRUMENT_SQUARE_FAST,
                BUILTIN_INSTRUMENT_TRIANGLE_FAST,
                BUILTIN_INSTRUMENT_BASS_DRUM,
                BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
                BUILTIN_INSTRUMENT_SNARE_DRUM_1,
            ]
        );
    }

    #[test]
    fn drum_aliases_in_drum_track_parse() {
        let table = parse_mml_to_compact_sequence_table(
            "#TRACK melody\nT120 L4 @0 c\n#TRACK drums\nT120 L16 !bd !hh !sd !oh !cy",
        )
        .unwrap();

        assert_eq!(table.tracks.len(), 2);
        assert!(table
            .instruments
            .iter()
            .any(|instrument| instrument.builtin_id == BUILTIN_INSTRUMENT_BASS_DRUM));
        assert!(table
            .instruments
            .iter()
            .any(|instrument| instrument.builtin_id == BUILTIN_INSTRUMENT_OPEN_HI_HAT));
    }

    #[test]
    fn too_many_tracks_are_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table(
                "#TRACK a\nc\n#TRACK b\nd\n#TRACK c\ne\n#TRACK d\nf\n#TRACK e\ng"
            ),
            Err(MmlParseError::TooManyTracks)
        );
    }

    #[test]
    fn empty_track_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("#TRACK melody\nc\n#TRACK bass\n   \n"),
            Err(MmlParseError::EmptyTrack)
        );
    }

    #[test]
    fn tempo_mismatch_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("#TRACK melody\nT120 c\n#TRACK bass\nT90 c"),
            Err(MmlParseError::TempoMismatch {
                expected: 120,
                actual: 90,
            })
        );
    }

    #[test]
    fn tempo_default_length_octave_note_and_rest_are_reflected() {
        let table = parse_mml_to_compact_sequence_table("T90 L8 O5 c r16").unwrap();

        assert_eq!(
            table.tempo,
            CompactTempo {
                tick_rate_hz: 6,
                bpm: 90,
                ticks_per_beat: 4,
            }
        );
        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(72).frequency_hz,
                duration_ticks: 2,
                volume: 127,
                instrument_id: 0,
            }
        );
        assert_eq!(
            table.tracks[0].events[1],
            CompactEvent::Rest { duration_ticks: 1 }
        );
    }

    #[test]
    fn semicolon_and_slash_comments_are_ignored() {
        let table = parse_mml_to_compact_sequence_table(
            "#TRACK melody ; lead line\n\
             T120 L4 O4 c // skip this tail\n\
             d ; and this tail\n\
             #TRACK bass // low line\n\
             T120 L4 O3 c r",
        )
        .unwrap();

        assert_eq!(table.tracks.len(), 2);
        assert_eq!(table.tracks[0].events.len(), 3);
        assert_eq!(table.tracks[1].events.len(), 3);
    }

    #[test]
    fn plus_and_hash_accidentals_are_sharps() {
        let table = parse_mml_to_compact_sequence_table("O4 c+ c#").unwrap();

        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(61).frequency_hz,
                duration_ticks: 4,
                volume: 127,
                instrument_id: 0,
            }
        );
        assert_eq!(table.tracks[0].events[0], table.tracks[0].events[1]);
    }

    #[test]
    fn accidentals_can_cross_octave_boundaries() {
        let table = parse_mml_to_compact_sequence_table("O4 b+ > c-").unwrap();

        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(72).frequency_hz,
                duration_ticks: 4,
                volume: 127,
                instrument_id: 0,
            }
        );
        assert_eq!(
            table.tracks[0].events[1],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(71).frequency_hz,
                duration_ticks: 4,
                volume: 127,
                instrument_id: 0,
            }
        );
    }

    #[test]
    fn dotted_note_duration_is_one_and_a_half_times_base_duration() {
        let table = parse_mml_to_compact_sequence_table("L8 c. d8.").unwrap();

        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(60).frequency_hz,
                duration_ticks: 3,
                volume: 127,
                instrument_id: 0,
            }
        );
        assert_eq!(
            table.tracks[0].events[1],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(62).frequency_hz,
                duration_ticks: 3,
                volume: 127,
                instrument_id: 0,
            }
        );
    }

    #[test]
    fn dotted_rest_duration_is_one_and_a_half_times_base_duration() {
        let table = parse_mml_to_compact_sequence_table("c r4.").unwrap();

        assert_eq!(
            table.tracks[0].events[1],
            CompactEvent::Rest { duration_ticks: 6 }
        );
    }

    #[test]
    fn unsupported_double_dot_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("c8.."),
            Err(MmlParseError::InvalidDuration)
        );
    }

    #[test]
    fn square_saw_and_bass_drum_builtin_ids_are_reflected() {
        let table = parse_mml_to_compact_sequence_table("@0 c @4 d @6 e").unwrap();

        assert_eq!(
            table.instruments[0].builtin_id,
            BUILTIN_INSTRUMENT_SQUARE_FAST
        );
        assert_eq!(table.instruments[1].builtin_id, BUILTIN_INSTRUMENT_SAW);
        assert_eq!(
            table.instruments[2].builtin_id,
            BUILTIN_INSTRUMENT_BASS_DRUM
        );
        assert_eq!(
            table.tracks[0].events[2],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(64).frequency_hz,
                duration_ticks: 4,
                volume: 127,
                instrument_id: 2,
            }
        );
    }

    #[test]
    fn unknown_or_reserved_instrument_id_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("@12 c"),
            Err(MmlParseError::UnknownInstrumentId(12))
        );
        assert_eq!(
            parse_mml_to_compact_sequence_table("@17 c"),
            Err(MmlParseError::UnknownInstrumentId(17))
        );
    }

    #[test]
    fn bass_drum_alias_parses_to_bass_drum_event() {
        let table = parse_mml_to_compact_sequence_table("!bd").unwrap();

        assert_eq!(
            table.instruments[0].builtin_id,
            BUILTIN_INSTRUMENT_BASS_DRUM
        );
        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(60).frequency_hz,
                duration_ticks: 4,
                volume: 127,
                instrument_id: 0,
            }
        );
    }

    #[test]
    fn drum_alias_explicit_duration_is_reflected() {
        let table = parse_mml_to_compact_sequence_table("L4 !hh16").unwrap();

        assert_eq!(
            table.instruments[0].builtin_id,
            BUILTIN_INSTRUMENT_CLOSED_HI_HAT
        );
        assert_eq!(
            table.tracks[0].events[0],
            CompactEvent::Note {
                pitch: SequencePitch::from_midi_note(60).frequency_hz,
                duration_ticks: 1,
                volume: 127,
                instrument_id: 0,
            }
        );
    }

    #[test]
    fn all_drum_aliases_map_to_m12_builtin_ids() {
        let table =
            parse_mml_to_compact_sequence_table("!bd !sd !s2 !hh !oh !cy !th !tm !tl !cl").unwrap();
        let builtin_ids: Vec<u8> = table
            .instruments
            .iter()
            .map(|instrument| instrument.builtin_id)
            .collect();

        assert_eq!(
            builtin_ids,
            vec![
                BUILTIN_INSTRUMENT_BASS_DRUM,
                BUILTIN_INSTRUMENT_SNARE_DRUM_1,
                BUILTIN_INSTRUMENT_SNARE_DRUM_2,
                BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
                BUILTIN_INSTRUMENT_OPEN_HI_HAT,
                BUILTIN_INSTRUMENT_CRASH_CYMBAL,
                BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH,
                BUILTIN_INSTRUMENT_SYNTH_TOM_MID,
                BUILTIN_INSTRUMENT_SYNTH_TOM_LOW,
                BUILTIN_INSTRUMENT_CLAP,
            ]
        );
    }

    #[test]
    fn drum_aliases_mixed_with_normal_notes_validate() {
        let table = parse_mml_to_compact_sequence_table("@0 c !hh16 d !sd").unwrap();
        let runtime_tracks: Vec<CompactTrack<'_>> = table
            .tracks
            .iter()
            .map(|track| CompactTrack::new(&track.events, track.gain, track.initial_instrument_id))
            .collect();

        assert_eq!(table.instruments.len(), 3);
        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &table.instruments,
                &runtime_tracks,
                table.tempo,
            )),
            Ok(())
        );
    }

    #[test]
    fn unknown_drum_alias_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("!zz"),
            Err(MmlParseError::UnknownDrumAlias("zz".to_string()))
        );
    }

    #[test]
    fn zero_duration_or_malformed_note_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("c0"),
            Err(MmlParseError::InvalidDuration)
        );
        assert_eq!(
            parse_mml_to_compact_sequence_table("c64"),
            Err(MmlParseError::InvalidDuration)
        );
        assert_eq!(
            parse_mml_to_compact_sequence_table("c16."),
            Err(MmlParseError::InvalidDuration)
        );
    }

    #[test]
    fn simple_loop_parses() {
        let table = parse_mml_to_compact_sequence_table("[c r]2").unwrap();

        assert_eq!(table.tracks[0].events[0], CompactEvent::LoopStart);
        assert_eq!(
            table.tracks[0].events[3],
            CompactEvent::LoopEnd { repeat_count: 1 }
        );
    }

    #[test]
    fn zero_loop_repeat_maps_to_infinite_sentinel() {
        let table = parse_mml_to_compact_sequence_table("[c]0").unwrap();

        assert_eq!(
            table.tracks[0].events[2],
            CompactEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE
            }
        );
    }

    #[test]
    fn nested_loop_is_rejected() {
        assert_eq!(
            parse_mml_to_compact_sequence_table("[c [d]2]2"),
            Err(MmlParseError::NestedLoop)
        );
    }

    #[test]
    fn generated_rust_fragment_validates() {
        let table = parse_mml_to_compact_sequence_table("T120 L8 O4 @3 c e !hh16 g r").unwrap();

        let output = format_compact_sequence_table(
            &table,
            CompactSequenceTableOptions {
                sequence_symbol_name: "MML_COMPACT".to_string(),
                table_symbol_prefix: "MML_COMPACT".to_string(),
            },
        )
        .unwrap();

        assert_eq!(output.instrument_count, 2);
        assert_eq!(output.track_count, 1);
        assert!(output.rust_fragment.contains("pub static MML_COMPACT"));
    }

    #[test]
    fn generated_multi_track_rust_fragment_validates() {
        let table = parse_mml_to_compact_sequence_table(
            "#TRACK melody\nT120 L8 O4 @3 c e g\n#TRACK bass\nT120 L4 O2 @5 c r",
        )
        .unwrap();

        let output = format_compact_sequence_table(
            &table,
            CompactSequenceTableOptions {
                sequence_symbol_name: "MML_MULTI_COMPACT".to_string(),
                table_symbol_prefix: "MML_MULTI_COMPACT".to_string(),
            },
        )
        .unwrap();

        assert_eq!(output.track_count, 2);
        assert!(output
            .rust_fragment
            .contains("pub static MML_MULTI_COMPACT_TRACKS: [CompactTrack<'static>; 2]"));
    }

    #[test]
    fn practical_mml_examples_parse_and_validate() {
        let examples = [
            (
                "blocks_like_bgm",
                include_str!("../../../examples/mml/blocks_like_bgm.mml"),
                3,
            ),
            (
                "line_clear_jingle",
                include_str!("../../../examples/mml/line_clear_jingle.mml"),
                3,
            ),
        ];

        for (name, source, expected_tracks) in examples {
            let table = parse_mml_to_compact_sequence_table(source).unwrap();

            assert_eq!(table.tracks.len(), expected_tracks, "{name}");
            assert!(!table.instruments.is_empty(), "{name}");
            assert!(
                table
                    .tracks
                    .iter()
                    .all(|track| matches!(track.events.last(), Some(CompactEvent::End))),
                "{name}"
            );
        }
    }

    #[test]
    fn practical_mml_example_generated_fragment_matches_checked_in_output() {
        let source = include_str!("../../../examples/mml/blocks_like_bgm.mml");
        let table = parse_mml_to_compact_sequence_table(source).unwrap();

        let output = format_compact_sequence_table(
            &table,
            CompactSequenceTableOptions {
                sequence_symbol_name: "BLOCKS_LIKE_BGM_COMPACT".to_string(),
                table_symbol_prefix: "BLOCKS_LIKE_BGM_COMPACT".to_string(),
            },
        )
        .unwrap();

        assert_eq!(output.instrument_count, 9);
        assert_eq!(output.track_count, 3);
        assert_eq!(output.event_counts, vec![33, 33, 35]);
        assert_eq!(
            output.rust_fragment,
            include_str!("../../../examples/generated/blocks_like_bgm.rs")
        );
    }

    #[test]
    fn parser_output_uses_runtime_validation() {
        let table = CompactSequenceTable {
            instruments: vec![CompactInstrument::builtin(
                BUILTIN_INSTRUMENT_SQUARE_FAST,
                255,
            )],
            tracks: vec![CompactSequenceTableTrack {
                events: vec![CompactEvent::Rest { duration_ticks: 0 }, CompactEvent::End],
                gain: MixerVolume::UNITY,
                initial_instrument_id: 0,
            }],
            tempo: CompactTempo::from_tick_rate_hz(4),
        };

        assert_eq!(
            format_compact_sequence_table(&table, CompactSequenceTableOptions::default()),
            Err(CompactSequenceTableError::RuntimeValidation(
                CompactSequenceError::ZeroDuration
            ))
        );
    }
}
