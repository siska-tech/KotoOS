//! Host-owned audio for KotoSim: a small KotoMML parser + square-wave synth and a
//! mixing engine fed by the audio host calls (KOTO-0095).
//!
//! # LEGACY (deprecated — KOTO-0162)
//!
//! This runtime KotoMML (`.kmml`) parser + synth is the **legacy** KotoOS audio path.
//! The **primary** path is the KotoAudio
//! generated-sequence runtime driven by [`crate::koto_blocks_audio`] (folded into
//! [`SimAudio::render`] via [`SimAudio::seq_start_bgm`] / [`SimAudio::seq_sfx`]). New
//! music/SFX target compact sequences / generated tables / built-in drums / the KACL
//! clip path — not this MML synth. Old KotoMML compatibility is **not** guaranteed;
//! see `docs/architecture/AUDIO_DEPRECATION_POLICY.md`.
//!
//! It remains only for low-level simulator compatibility tests; package assets use
//! Native KotoAudio runtime cues or KACL clips.
//!
//! Per [`docs/spec/KOTOMML_FORMAT.md`] and the audio design, MML synthesis is host-owned:
//! Koto apps trigger sound *by id* (`play_sfx` / `play_bgm`) and the host renders the
//! PCM, so the 4 KB VM heap never holds waveforms. The low-level `audio_submit_i16`
//! primitive still streams raw app PCM through the same [`SimAudio`] mixer.
//!
//! koto-core is `no_std` without `alloc`, so this synth (which uses `Vec`) lives in
//! koto-sim. It reuses the saturation model of [`koto_core::PcmMixer`]; the engine
//! is deterministic so scripted/golden runs can capture audio without a device.

use std::collections::VecDeque;
use std::path::Path;

use koto_audio::{
    AudioLimits, ClipAssetHeader, DecodeResult, MixerVolume, OwnedClipPlayer, RuntimeCue,
    RuntimeCuePlayer, StreamingClipDecoder, CLIP_ASSET_HEADER_SIZE,
};

use crate::koto_blocks_audio::KotoBlocksAudio;

/// Output sample rate for the headless/default engine. Window mode overrides this
/// with the audio device's actual rate (no resampling).
pub const DEFAULT_SAMPLE_RATE: u32 = 22_050;

/// Maximum simultaneously-rendering one-shot SFX voices (KOTOMML_FORMAT.md budget).
pub const MAX_SFX_VOICES: usize = 2;

/// Maximum simultaneous BGM melodic voices (a multi-track score beyond this is
/// truncated). Sized to the RP2040 CPU budget in KOTOMML_FORMAT.md.
pub const MAX_BGM_VOICES: usize = 4;

/// Ticks per quarter note in the KotoMML event model.
const TICKS_PER_QUARTER: u32 = 96;

/// Per-voice peak amplitude at full volume, leaving mixing headroom below the i16
/// ceiling so layered BGM + SFX rarely saturate.
const VOICE_PEAK: f64 = 8_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmlError {
    UnknownCommand,
    BadNumber,
    BadLength,
    OctaveRange,
    NoteRange,
    /// An `@n` selected an instrument id outside the bank. Only reported in strict
    /// parsing; lenient playback falls back to `@0` (see [`instrument`]).
    UnknownInstrument,
    /// A `]` loop-end without a matching `[`.
    UnmatchedLoop,
}

/// Number of built-in instruments in the host bank (`@0`..`@5`); see [`instrument`].
pub const INSTRUMENT_COUNT: u8 = 6;

/// A parsed KotoMML event with its duration already resolved to samples at the
/// parse-time sample rate (notes create a voice; rests only advance the cursor).
/// `instrument` selects the host synth voice (waveform + envelope), set by the
/// `@n` command (see [`instrument`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmlEvent {
    pub is_note: bool,
    pub midi_note: u8,
    pub samples: u32,
    pub volume: u8,
    pub instrument: u8,
}

/// A parsed KotoMML track: the timed event list plus the loop region a looping
/// voice (BGM) repeats. `loop_start..loop_end` is the body to repeat; events before
/// `loop_start` are a one-shot intro. With no `[`/`]` the whole track loops.
///
/// `bank` carries any package-defined custom wavetable instruments (`@n` ids at or
/// above [`CUSTOM_INSTRUMENT_MIN`]); every track in a score shares the same bank so a
/// looping voice can resolve its instrument while it plays. It is `Eq`-incompatible
/// (wavetables are `f64`), so this type is only `PartialEq`.
#[derive(Clone, Debug, PartialEq)]
pub struct MmlTrack {
    pub events: Vec<MmlEvent>,
    pub loop_start: usize,
    pub loop_end: usize,
}

/// Parse a KotoMML track into timed events at `sample_rate`. Unknown `@n`
/// instruments fall back to `@0` at playback; use [`parse_mml_strict`] to reject
/// them. See [`MmlTrack`] for the loop model.
pub fn parse_mml(text: &str, sample_rate: u32) -> Result<MmlTrack, MmlError> {
    parse_inner(text, sample_rate, false)
}

/// Like [`parse_mml`] but rejects an `@n` outside the instrument bank
/// ([`MmlError::UnknownInstrument`]). Used by tests and asset tooling to catch typos
/// the lenient player would silently fall back on.
pub fn parse_mml_strict(text: &str, sample_rate: u32) -> Result<MmlTrack, MmlError> {
    parse_inner(text, sample_rate, true)
}

/// Parse a multi-track KotoMML score into one [`MmlTrack`] per voice. Tracks are
/// separated by `#TRACK` marker lines (case-insensitive); each section is an
/// independent track with its own tempo, octave, instrument, and loop. Text with no
/// `#TRACK` marker is a single track, so this is a superset of [`parse_mml`]. Empty
/// (comment-only) sections are dropped.
pub fn parse_mml_multi(text: &str, sample_rate: u32) -> Result<Vec<MmlTrack>, MmlError> {
    parse_multi(text, sample_rate, false)
}

/// Strict [`parse_mml_multi`]: rejects unknown `@n` instruments (for tests / tooling).
pub fn parse_mml_multi_strict(text: &str, sample_rate: u32) -> Result<Vec<MmlTrack>, MmlError> {
    parse_multi(text, sample_rate, true)
}

/// A `#TRACK` marker line begins a new voice (the keyword must stand alone, so an
/// ordinary `# comment` is not mistaken for one).
fn is_track_marker(line: &str) -> bool {
    let bytes = line.trim_start().as_bytes();
    bytes.len() >= 6
        && bytes[..6].eq_ignore_ascii_case(b"#track")
        && (bytes.len() == 6 || bytes[6].is_ascii_whitespace())
}

fn parse_multi(text: &str, sample_rate: u32, strict: bool) -> Result<Vec<MmlTrack>, MmlError> {
    let mut chunks: Vec<String> = vec![String::new()];
    for line in text.lines() {
        if is_track_marker(line) {
            chunks.push(String::new());
        } else {
            let chunk = chunks.last_mut().expect("at least one chunk");
            chunk.push_str(line);
            chunk.push('\n');
        }
    }
    let mut tracks = Vec::new();
    for chunk in &chunks {
        if chunk.trim().is_empty() {
            continue;
        }
        let track = parse_inner(chunk, sample_rate, strict)?;
        if !track.events.is_empty() {
            tracks.push(track);
        }
    }
    Ok(tracks)
}

/// Implements the v0 subset from `docs/spec/KOTOMML_FORMAT.md`: notes `A`–`G` with
/// `#`/`+`/`-` and an optional length + dot, rests `R`, the `T`/`V`/`O`/`L`/`>`/`<`
/// commands, the `@n` instrument select, and `[`/`]` loop markers.
fn parse_inner(text: &str, sample_rate: u32, strict: bool) -> Result<MmlTrack, MmlError> {
    let mut tempo: u32 = 120;
    let mut volume: u8 = 10;
    let mut octave: i32 = 4;
    let mut length: u32 = 4;
    let mut current_instrument: u8 = 0;
    let mut loop_start: usize = 0;
    let mut loop_end: Option<usize> = None;
    let mut saw_loop_open = false;
    let mut events: Vec<MmlEvent> = Vec::new();
    // Fractional-sample accumulator so repeated short notes do not drift.
    let mut carry: f64 = 0.0;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
                continue;
            }
            let cmd = c.to_ascii_uppercase();
            chars.next();
            match cmd {
                'T' => tempo = read_number(&mut chars)?.clamp(32, 255),
                'V' => volume = read_number(&mut chars)?.min(15) as u8,
                'O' => {
                    let o = read_number(&mut chars)? as i32;
                    if !(0..=8).contains(&o) {
                        return Err(MmlError::OctaveRange);
                    }
                    octave = o;
                }
                'L' => length = validate_length(read_number(&mut chars)?)?,
                '@' => {
                    let id = read_number(&mut chars)?;
                    // Valid if it is a built-in voice or a custom id the bank defines;
                    // strict parsing rejects anything else so a typo (or a missing
                    // definition surfaces instead of silently falling back to `@0`.
                    let known = id < INSTRUMENT_COUNT as u32;
                    if strict && !known {
                        return Err(MmlError::UnknownInstrument);
                    }
                    current_instrument = id.min(u8::MAX as u32) as u8;
                }
                '[' => {
                    loop_start = events.len();
                    saw_loop_open = true;
                }
                ']' => loop_end = Some(events.len()),
                '>' => octave = (octave + 1).min(8),
                '<' => octave = (octave - 1).max(0),
                'R' => {
                    let (ticks, _) = read_duration(&mut chars, length)?;
                    let samples = ticks_to_samples(ticks, tempo, sample_rate, &mut carry);
                    events.push(MmlEvent {
                        is_note: false,
                        midi_note: 0,
                        samples,
                        volume,
                        instrument: current_instrument,
                    });
                }
                'A'..='G' => {
                    let mut semitone = note_semitone(cmd);
                    // Accidental.
                    match chars.peek() {
                        Some('#') | Some('+') => {
                            chars.next();
                            semitone += 1;
                        }
                        Some('-') => {
                            chars.next();
                            semitone -= 1;
                        }
                        _ => {}
                    }
                    let midi = (octave + 1) * 12 + semitone;
                    if !(0..=127).contains(&midi) {
                        return Err(MmlError::NoteRange);
                    }
                    let (ticks, _) = read_duration(&mut chars, length)?;
                    let samples = ticks_to_samples(ticks, tempo, sample_rate, &mut carry);
                    events.push(MmlEvent {
                        is_note: true,
                        midi_note: midi as u8,
                        samples,
                        volume,
                        instrument: current_instrument,
                    });
                }
                _ => return Err(MmlError::UnknownCommand),
            }
        }
    }

    // A `]` without a `[` is malformed; otherwise the loop body ends at `]` (or the
    // track end), and `loop_start` is `[` (or the track start).
    if loop_end.is_some() && !saw_loop_open {
        return Err(MmlError::UnmatchedLoop);
    }
    let loop_end = loop_end.unwrap_or(events.len());
    Ok(MmlTrack {
        events,
        loop_start,
        loop_end,
    })
}

fn note_semitone(letter: char) -> i32 {
    match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => 0,
    }
}

fn read_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<u32, MmlError> {
    let mut digits = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            digits.push(c);
            chars.next();
        } else {
            break;
        }
    }
    digits.parse::<u32>().map_err(|_| MmlError::BadNumber)
}

/// Read an optional explicit note/rest length and dot, returning `(ticks, dotted)`.
fn read_duration(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    default_length: u32,
) -> Result<(u32, bool), MmlError> {
    let length = if matches!(chars.peek(), Some(c) if c.is_ascii_digit()) {
        validate_length(read_number(chars)?)?
    } else {
        default_length
    };
    let mut ticks = TICKS_PER_QUARTER * 4 / length;
    let dotted = matches!(chars.peek(), Some('.'));
    if dotted {
        chars.next();
        ticks = ticks * 3 / 2;
    }
    Ok((ticks, dotted))
}

fn validate_length(length: u32) -> Result<u32, MmlError> {
    match length {
        1 | 2 | 4 | 8 | 16 | 32 => Ok(length),
        _ => Err(MmlError::BadLength),
    }
}

/// Convert a tick duration to samples at `tempo`/`sample_rate`, carrying the
/// fractional remainder so a run of short notes stays on the beat.
fn ticks_to_samples(ticks: u32, tempo: u32, sample_rate: u32, carry: &mut f64) -> u32 {
    let quarter_samples = sample_rate as f64 * 60.0 / tempo as f64;
    let exact = quarter_samples * (ticks as f64 / TICKS_PER_QUARTER as f64) + *carry;
    let whole = exact.floor();
    *carry = exact - whole;
    whole as u32
}

/// An oscillator shape for a synth voice. `Pulse*` are narrow-duty squares (thinner,
/// brighter than a 50% `Square`); `Noise` is a pitched-irrelevant LFSR hiss used for
/// percussive cues (lock, drums).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Waveform {
    Square,
    Pulse25,
    Pulse12,
    Triangle,
    Saw,
    Noise,
}

/// A simple ADSR amplitude envelope. Attack/decay/release are in milliseconds;
/// `sustain` is the held level in `[0, 1]` after the decay.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Adsr {
    attack_ms: u16,
    decay_ms: u16,
    sustain: f64,
    release_ms: u16,
}

/// A host synth voice definition: an oscillator shape, an ADSR envelope, and an
/// output gain. Selected per note by the `@n` KotoMML command.
#[derive(Clone, Copy, Debug)]
struct Instrument {
    wave: Waveform,
    adsr: Adsr,
    gain: f64,
}

/// The host's built-in instrument bank, indexed by the `@n` KotoMML command. Unknown
/// ids fall back to the `@0` lead. Keep this small and chiptune-flavored:
/// `0` square lead, `1` thin pulse, `2` soft triangle, `3` saw, `4` noise/drum,
/// `5` short pluck.
fn instrument(id: u8) -> Instrument {
    match id {
        1 => Instrument {
            wave: Waveform::Pulse25,
            adsr: Adsr {
                attack_ms: 1,
                decay_ms: 20,
                sustain: 0.6,
                release_ms: 25,
            },
            gain: 0.9,
        },
        2 => Instrument {
            wave: Waveform::Triangle,
            adsr: Adsr {
                attack_ms: 4,
                decay_ms: 60,
                sustain: 0.85,
                release_ms: 80,
            },
            gain: 1.1,
        },
        3 => Instrument {
            wave: Waveform::Saw,
            adsr: Adsr {
                attack_ms: 2,
                decay_ms: 30,
                sustain: 0.5,
                release_ms: 40,
            },
            gain: 0.7,
        },
        4 => Instrument {
            wave: Waveform::Noise,
            // Percussive: snap to zero sustain so it reads as a hit, not a tone.
            adsr: Adsr {
                attack_ms: 0,
                decay_ms: 35,
                sustain: 0.0,
                release_ms: 20,
            },
            gain: 1.0,
        },
        5 => Instrument {
            wave: Waveform::Pulse12,
            adsr: Adsr {
                attack_ms: 1,
                decay_ms: 15,
                sustain: 0.3,
                release_ms: 20,
            },
            gain: 0.8,
        },
        // `0` and anything unknown: the default square lead.
        _ => Instrument {
            wave: Waveform::Square,
            adsr: Adsr {
                attack_ms: 2,
                decay_ms: 40,
                sustain: 0.7,
                release_ms: 30,
            },
            gain: 1.0,
        },
    }
}

// Retained only as unreachable source history while old issue documents are being
// archived. `cfg(any())` deliberately compiles this former format support out.
#[cfg(any())]
mod retired_kwt {
    use super::*;
    use std::collections::BTreeMap;

    const CUSTOM_INSTRUMENT_MIN: u8 = INSTRUMENT_COUNT;
    const CUSTOM_INSTRUMENT_MAX: u8 = 31;
    const MAX_CUSTOM_INSTRUMENTS: usize = 8;
    const MIN_WAVETABLE_LEN: usize = 2;
    const MAX_WAVETABLE_LEN: usize = 64;

    /// A package-defined custom instrument: a single-cycle wavetable plus the same ADSR
    /// envelope and gain a built-in voice carries. Selected by `@n` for ids at or above
    /// [`CUSTOM_INSTRUMENT_MIN`] once the package's `.kwt` asset is loaded into the bank.
    #[derive(Clone, Debug, PartialEq)]
    struct VoiceDef {
        /// One period of the waveform, normalised to `[-1, 1]`; sampled by phase with
        /// linear interpolation (see [`table_sample`]).
        table: Vec<f64>,
        adsr: Adsr,
        gain: f64,
    }

    /// A bank of package-defined custom wavetable instruments, keyed by `@n` id. Built by
    /// the host from the `.kwt` assets a score references (see [`scan_instrument_refs`])
    /// and handed to the players via [`parse_mml_multi_banked`]. The built-in bank
    /// ([`instrument`]) is unaffected: an id this bank does not define falls through to it.
    ///
    /// Keeping the wavetable *data* in the package (the `.kwt` asset) rather than the host
    /// is the whole point of the extension — a package ships its own timbres.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct InstrumentBank {
        custom: BTreeMap<u8, VoiceDef>,
    }

    impl InstrumentBank {
        pub fn new() -> Self {
            Self {
                custom: BTreeMap::new(),
            }
        }

        /// Number of custom instruments defined.
        pub fn len(&self) -> usize {
            self.custom.len()
        }

        pub fn is_empty(&self) -> bool {
            self.custom.is_empty()
        }

        /// Define custom instrument `id` from the text of a KotoWaveTable (`.kwt`) asset.
        /// `id` must be in `CUSTOM_INSTRUMENT_MIN..=CUSTOM_INSTRUMENT_MAX`; the bank is
        /// capped at [`MAX_CUSTOM_INSTRUMENTS`]. Returns [`MmlError::BadInstrument`] on a
        /// malformed asset or an out-of-range id.
        pub fn define_from_kwt(&mut self, id: u8, text: &str) -> Result<(), MmlError> {
            if !(CUSTOM_INSTRUMENT_MIN..=CUSTOM_INSTRUMENT_MAX).contains(&id) {
                return Err(MmlError::BadInstrument);
            }
            if self.custom.len() >= MAX_CUSTOM_INSTRUMENTS && !self.custom.contains_key(&id) {
                return Err(MmlError::BadInstrument);
            }
            self.custom.insert(id, parse_kwt(text)?);
            Ok(())
        }
    }

    /// Parse a KotoWaveTable (`.kwt`) asset into a [`VoiceDef`]. The format is small,
    /// line-oriented, and case-insensitive (see `docs/spec/KOTOMML_FORMAT.md`):
    ///
    /// ```text
    /// KWT1
    /// WAVE 0 31 63 31 0 -31 -63 -31     # 2..64 ints in -100..100, one period
    /// ENV 4 60 70 90                    # optional: attack/decay ms, sustain %, release ms
    /// GAIN 120                          # optional: output gain %, default 100
    /// ```
    ///
    /// Blank lines and `#` comments are ignored. `WAVE` is required; `ENV`/`GAIN` default
    /// to a soft lead. Wave samples are normalised by 100 and clamped to `[-1, 1]`.
    fn parse_kwt(text: &str) -> Result<VoiceDef, MmlError> {
        let mut saw_magic = false;
        let mut table: Option<Vec<f64>> = None;
        // Default envelope/gain: a gentle lead so a `WAVE`-only definition still sounds.
        let mut adsr = Adsr {
            attack_ms: 2,
            decay_ms: 40,
            sustain: 0.7,
            release_ms: 30,
        };
        let mut gain = 1.0;

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = line.split_whitespace();
            let Some(keyword) = tokens.next() else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("KWT1") {
                saw_magic = true;
                continue;
            }
            if !saw_magic {
                // The magic header must come first so a stray file is not misread.
                return Err(MmlError::BadInstrument);
            }
            if keyword.eq_ignore_ascii_case("WAVE") {
                let mut samples = Vec::new();
                for token in tokens {
                    let value: i32 = token.parse().map_err(|_| MmlError::BadInstrument)?;
                    samples.push((value.clamp(-100, 100) as f64) / 100.0);
                }
                if !(MIN_WAVETABLE_LEN..=MAX_WAVETABLE_LEN).contains(&samples.len()) {
                    return Err(MmlError::BadInstrument);
                }
                table = Some(samples);
            } else if keyword.eq_ignore_ascii_case("ENV") {
                let nums = read_kwt_numbers(tokens, 4)?;
                adsr = Adsr {
                    attack_ms: nums[0].min(u16::MAX as i32).max(0) as u16,
                    decay_ms: nums[1].min(u16::MAX as i32).max(0) as u16,
                    sustain: (nums[2].clamp(0, 100) as f64) / 100.0,
                    release_ms: nums[3].min(u16::MAX as i32).max(0) as u16,
                };
            } else if keyword.eq_ignore_ascii_case("GAIN") {
                let nums = read_kwt_numbers(tokens, 1)?;
                gain = (nums[0].clamp(0, 400) as f64) / 100.0;
            } else {
                return Err(MmlError::BadInstrument);
            }
        }

        match table {
            Some(table) => Ok(VoiceDef { table, adsr, gain }),
            None => Err(MmlError::BadInstrument),
        }
    }

    /// Read exactly `count` whitespace-separated integers from a `.kwt` directive's
    /// remaining tokens; any shortfall, surplus, or non-integer is a malformed asset.
    fn read_kwt_numbers<'a>(
        tokens: impl Iterator<Item = &'a str>,
        count: usize,
    ) -> Result<Vec<i32>, MmlError> {
        let nums: Vec<i32> = tokens
            .map(|token| token.parse::<i32>().map_err(|_| MmlError::BadInstrument))
            .collect::<Result<_, _>>()?;
        if nums.len() != count {
            return Err(MmlError::BadInstrument);
        }
        Ok(nums)
    }

    /// Extract `#INST <id> <path>` directives from a KotoMML score: each binds a custom
    /// `@n` id to a package KotoWaveTable (`.kwt`) asset the host must load. Returns the
    /// `(id, path)` pairs in order. The host loads each path and calls
    /// [`InstrumentBank::define_from_kwt`]; the MML parser itself treats `#INST` lines as
    /// comments. A malformed directive (bad id, missing path, surplus tokens, or more than
    /// [`MAX_CUSTOM_INSTRUMENTS`]) is [`MmlError::BadInstrument`].
    pub fn scan_instrument_refs(mml: &str) -> Result<Vec<(u8, String)>, MmlError> {
        let mut refs = Vec::new();
        for raw_line in mml.lines() {
            let line = raw_line.trim();
            if !is_inst_marker(line) {
                continue;
            }
            let mut tokens = line.split_whitespace();
            tokens.next(); // "#INST"
            let id_token = tokens.next().ok_or(MmlError::BadInstrument)?;
            let path = tokens.next().ok_or(MmlError::BadInstrument)?;
            if tokens.next().is_some() {
                return Err(MmlError::BadInstrument);
            }
            let id: u8 = id_token.parse().map_err(|_| MmlError::BadInstrument)?;
            if !(CUSTOM_INSTRUMENT_MIN..=CUSTOM_INSTRUMENT_MAX).contains(&id) {
                return Err(MmlError::BadInstrument);
            }
            if refs.len() >= MAX_CUSTOM_INSTRUMENTS {
                return Err(MmlError::BadInstrument);
            }
            refs.push((id, path.to_string()));
        }
        Ok(refs)
    }

    /// A `#INST` directive line binds a custom instrument id to a `.kwt` asset. Like
    /// [`is_track_marker`], the keyword must stand alone so an ordinary `# comment` is not
    /// mistaken for one.
    fn is_inst_marker(line: &str) -> bool {
        let bytes = line.trim_start().as_bytes();
        bytes.len() >= 5
            && bytes[..5].eq_ignore_ascii_case(b"#inst")
            && (bytes.len() == 5 || bytes[5].is_ascii_whitespace())
    }

    /// Sample a single-cycle wavetable at `phase` in `[0, 1)` with linear interpolation
    /// between adjacent samples (wrapping at the period boundary).
    fn table_sample(table: &[f64], phase: f64) -> f64 {
        let n = table.len();
        if n == 0 {
            return 0.0;
        }
        let pos = phase * n as f64;
        let base = pos.floor();
        let i0 = (base as usize) % n;
        let i1 = (i0 + 1) % n;
        let frac = pos - base;
        table[i0] * (1.0 - frac) + table[i1] * frac
    }
}

/// A monophonic synth voice that renders an event list to i16 samples. Each note
/// uses the instrument its `@n` selected (waveform + ADSR). A looping voice (BGM)
/// repeats its `loop_start..loop_end` body forever after the intro; a non-looping
/// voice (SFX) plays once to the end.
#[derive(Clone, Debug)]
pub struct MmlPlayer {
    events: Vec<MmlEvent>,
    sample_rate: u32,
    looping: bool,
    loop_start: usize,
    loop_end: usize,
    index: usize,
    cursor: u32,
    phase: f64,
    /// Deterministic LFSR state for the noise oscillator (fixed seed → reproducible
    /// capture).
    noise: u32,
    done: bool,
}

impl MmlPlayer {
    pub fn new(track: MmlTrack, sample_rate: u32, looping: bool) -> Self {
        let done = track.events.is_empty();
        Self {
            events: track.events,
            sample_rate,
            looping,
            loop_start: track.loop_start,
            loop_end: track.loop_end,
            index: 0,
            cursor: 0,
            phase: 0.0,
            noise: 0x1234_5678,
            done,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Render and advance one sample. Returns silence (`0`) once a non-looping voice
    /// is finished.
    pub fn next_sample(&mut self) -> i16 {
        if self.done {
            return 0;
        }
        // A looping voice repeats `loop_start..loop_end`; a one-shot voice ends at the
        // track end. Skip past any zero-length events, bounded so an all-zero-length
        // loop body cannot spin forever.
        let mut guard = 0;
        loop {
            let boundary = if self.looping {
                self.loop_end.min(self.events.len())
            } else {
                self.events.len()
            };
            if self.index >= boundary {
                if self.looping && self.loop_start < boundary {
                    self.index = self.loop_start;
                    self.cursor = 0;
                    self.phase = 0.0;
                } else {
                    self.done = true;
                    return 0;
                }
            }
            if self.cursor < self.current_samples() {
                break;
            }
            self.index += 1;
            self.cursor = 0;
            self.phase = 0.0;
            guard += 1;
            if guard > self.events.len() + 1 {
                self.done = true;
                return 0;
            }
        }

        let event = self.events[self.index];
        let value = if event.is_note {
            let freq = 440.0 * 2f64.powf((event.midi_note as f64 - 69.0) / 12.0);
            self.phase += freq / self.sample_rate as f64;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let instr = instrument(event.instrument);
            let wave = waveform_sample(instr.wave, self.phase, &mut self.noise);
            let adsr = instr.adsr;
            let gain = instr.gain;
            let env = adsr_envelope(&adsr, self.cursor, event.samples, self.sample_rate);
            (wave * env * (event.volume as f64 / 15.0) * VOICE_PEAK * gain) as i32
        } else {
            0
        };
        self.cursor += 1;
        clamp_i16(value)
    }

    fn current_samples(&self) -> u32 {
        self.events.get(self.index).map(|e| e.samples).unwrap_or(0)
    }
}

/// One oscillator sample in `[-1, 1]` for `wave` at `phase` in `[0, 1)`. `noise`
/// carries the LFSR state for [`Waveform::Noise`].
fn waveform_sample(wave: Waveform, phase: f64, noise: &mut u32) -> f64 {
    match wave {
        Waveform::Square => sign(phase < 0.5),
        Waveform::Pulse25 => sign(phase < 0.25),
        Waveform::Pulse12 => sign(phase < 0.125),
        // Rises -1 → 1 over the first half, falls back over the second.
        Waveform::Triangle => {
            if phase < 0.5 {
                phase * 4.0 - 1.0
            } else {
                3.0 - phase * 4.0
            }
        }
        Waveform::Saw => 2.0 * phase - 1.0,
        Waveform::Noise => {
            *noise = lfsr_next(*noise);
            sign(*noise & 1 == 0)
        }
    }
}

fn sign(positive: bool) -> f64 {
    if positive {
        1.0
    } else {
        -1.0
    }
}

/// Advance a 32-bit xorshift LFSR (deterministic white-noise source).
fn lfsr_next(state: u32) -> u32 {
    let mut x = state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

/// ADSR amplitude in `[0, 1]` for a note of `total` samples at position `pos`. The
/// attack+decay are squeezed to fit before the release window so very short notes
/// still open and close cleanly.
fn adsr_envelope(adsr: &Adsr, pos: u32, total: u32, sample_rate: u32) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let ms_to_samples = |ms: u16| ((sample_rate as u64 * ms as u64) / 1000) as u32;
    let mut attack = ms_to_samples(adsr.attack_ms);
    let mut decay = ms_to_samples(adsr.decay_ms);
    let release = ms_to_samples(adsr.release_ms).min(total);
    let pre_release = total - release;
    if attack + decay > pre_release {
        let scale = pre_release as f64 / (attack + decay).max(1) as f64;
        attack = (attack as f64 * scale) as u32;
        decay = (decay as f64 * scale) as u32;
    }
    let release_start = total - release;
    if pos < attack {
        pos as f64 / attack.max(1) as f64
    } else if pos < attack + decay {
        let t = (pos - attack) as f64 / decay.max(1) as f64;
        1.0 - (1.0 - adsr.sustain) * t
    } else if pos < release_start {
        adsr.sustain
    } else if release == 0 {
        0.0
    } else {
        adsr.sustain * (total - pos) as f64 / release as f64
    }
}

fn clamp_i16(sample: i32) -> i16 {
    sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// A per-frame audio action recorded by the host, for deterministic inspection of
/// scripted runs (mirrors the recorded draw lists).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEvent {
    Sfx(i32),
    SfxAsset,
    Bgm(i32),
    BgmAsset,
    StopBgm,
    SubmitPcm { frames: i32, channels: i32 },
}

/// Default mix gains. BGM sits below unity so one-shot SFX (line clear, lock) cut
/// through it rather than being buried; raw submitted PCM plays at unity.
pub const DEFAULT_BGM_GAIN: f64 = 0.55;
pub const DEFAULT_SFX_GAIN: f64 = 1.0;

/// Bounded Native KMML runtime shapes shared with the device path.
pub const RUNTIME_BGM_EVENTS_PER_TRACK: usize = 272;
pub const RUNTIME_SFX_EVENTS_PER_TRACK: usize = 32;
/// Maximum complete KACL image accepted from one package asset.
pub const RUNTIME_CLIP_IMAGE_CAPACITY: usize = 8192;
const RUNTIME_SFX_PLAYERS: usize = 3;

/// The host-owned mixing engine. It carries two paths in one output stream:
///
/// * **Primary (KOTO-0162):** the KotoAudio generated-sequence bridge (`seq`), driven
///   by [`seq_start_bgm`](Self::seq_start_bgm) / [`seq_sfx`](Self::seq_sfx) and folded
///   in at the end of [`render`](Self::render).
/// * **Legacy (deprecated):** the multi-voice looping MML BGM (`bgm`), the bounded
///   one-shot MML SFX voices (`sfx`), and the raw-PCM queue fed by `audio_submit_i16`
///   (`raw`). New audio should use the primary path.
///
/// [`render`](Self::render) is called by the cpal callback (window mode) or the
/// headless capture path.
#[derive(Debug)]
pub struct SimAudio {
    sample_rate: u32,
    bgm: Vec<MmlPlayer>,
    sfx: Vec<MmlPlayer>,
    raw: VecDeque<i16>,
    bgm_gain: f64,
    sfx_gain: f64,
    /// Optional koto-audio generated-sequence bridge (KotoBlocks BGM/SFX). Lazily
    /// created at the current sample rate on first use; its mixed output is folded
    /// into [`render`](Self::render). `None` for apps that never use it, so the
    /// existing MML/raw paths are unaffected.
    seq: Option<KotoBlocksAudio>,
    /// SD-loaded Native KMML compiled into owned KotoAudio event storage.
    runtime_bgm: RuntimeCuePlayer<RUNTIME_BGM_EVENTS_PER_TRACK>,
    runtime_sfx: [RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK>; RUNTIME_SFX_PLAYERS],
    runtime_sfx_cursor: usize,
    runtime_clip: OwnedClipPlayer<RUNTIME_CLIP_IMAGE_CAPACITY>,
    runtime_stream: Option<SimStreamingClip>,
    runtime_clip_bgm: bool,
}

#[derive(Debug)]
struct SimStreamingClip {
    image: Vec<u8>,
    payload_start: usize,
    payload_cursor: usize,
    decoder: StreamingClipDecoder,
    pass_header: ClipAssetHeader,
    looping: bool,
}

impl SimStreamingClip {
    fn new(image: Vec<u8>, _sample_rate: u32) -> Option<Self> {
        let header = ClipAssetHeader::decode(image.get(..CLIP_ASSET_HEADER_SIZE)?).ok()?;
        let payload_start = usize::from(header.header_size);
        let payload_end = payload_start.checked_add(header.payload_size as usize)?;
        if payload_end != image.len() {
            return None;
        }
        let mut limits = AudioLimits::v0_default();
        // KACL assets carry their own device-rate contract. The simulator
        // currently advances one clip sample per output sample; accepting the
        // encoded rate here keeps package playback available on host devices
        // whose callback rate differs from PicoCalc's 16 kHz mixer.
        limits.sample_rate_hz = header.sample_rate_hz;
        let looping = matches!(
            header.loop_metadata().ok()?,
            koto_audio::ClipLoop::Whole {
                count: koto_audio::LoopCount::Infinite
            }
        );
        if !looping && !matches!(header.loop_metadata().ok()?, koto_audio::ClipLoop::None) {
            return None;
        }
        let mut pass_header = header;
        pass_header.loop_start = 0;
        pass_header.loop_end = 0;
        pass_header.loop_count = 0;
        let decoder = StreamingClipDecoder::from_header(pass_header, limits).ok()?;
        Some(Self {
            image,
            payload_start,
            payload_cursor: 0,
            decoder,
            pass_header,
            looping,
        })
    }

    fn next_sample(&mut self) -> DecodeResult {
        let mut out = [0i16; 1];
        let payload = &self.image[self.payload_start + self.payload_cursor..];
        let (consumed, written) = self.decoder.decode_chunk(payload, &mut out);
        self.payload_cursor += consumed;
        if written == 1 {
            DecodeResult::Sample(out[0])
        } else if self.looping && self.decoder.is_finished() {
            self.payload_cursor = 0;
            self.decoder = StreamingClipDecoder::from_header(
                self.pass_header,
                AudioLimits {
                    sample_rate_hz: self.pass_header.sample_rate_hz,
                    ..AudioLimits::v0_default()
                },
            )
            .expect("validated streaming loop header");
            self.next_sample()
        } else {
            DecodeResult::End
        }
    }
}

impl SimAudio {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            bgm: Vec::new(),
            sfx: Vec::new(),
            raw: VecDeque::new(),
            bgm_gain: DEFAULT_BGM_GAIN,
            sfx_gain: DEFAULT_SFX_GAIN,
            seq: None,
            runtime_bgm: RuntimeCuePlayer::new(sample_rate.max(1)),
            runtime_sfx: [RuntimeCuePlayer::new(sample_rate.max(1)); RUNTIME_SFX_PLAYERS],
            runtime_sfx_cursor: 0,
            runtime_clip: OwnedClipPlayer::new(),
            runtime_stream: None,
            runtime_clip_bgm: false,
        }
    }

    /// Lazily creates the koto-audio sequence bridge at the current sample rate.
    fn ensure_seq(&mut self) -> Option<&mut KotoBlocksAudio> {
        if self.seq.is_none() {
            self.seq = KotoBlocksAudio::new(self.sample_rate);
        }
        self.seq.as_mut()
    }

    /// Starts (once) the KotoBlocks generated-sequence BGM. Idempotent: repeated
    /// calls while playing do not restart or stack the loop.
    pub fn seq_start_bgm_cue(
        &mut self,
        sequence: &'static koto_audio::PolyphonicSequence<'static>,
    ) {
        if let Some(seq) = self.ensure_seq() {
            seq.start_bgm_cue(sequence);
        }
    }

    /// Stops only the sequence BGM bus (sequence SFX keep playing).
    pub fn seq_stop_bgm(&mut self) {
        if let Some(seq) = &mut self.seq {
            seq.stop_bgm();
        }
        self.runtime_bgm.stop();
    }

    /// Triggers a KotoBlocks generated-sequence SFX cue.
    pub fn seq_sfx_cue(&mut self, sequence: &'static koto_audio::Sequence<'static>) {
        if let Some(seq) = self.ensure_seq() {
            seq.sfx_cue(sequence);
        }
    }

    /// Starts a Native KMML cue that was compiled from the mounted SD asset.
    pub fn play_runtime_bgm(&mut self, cue: RuntimeCue<RUNTIME_BGM_EVENTS_PER_TRACK>) {
        let _ = self.runtime_bgm.play(cue);
    }

    /// Starts an owned Native KMML one-shot, using a bounded overlap pool.
    pub fn play_runtime_sfx(&mut self, cue: RuntimeCue<RUNTIME_SFX_EVENTS_PER_TRACK>) {
        let slot = self
            .runtime_sfx
            .iter()
            .position(|player| !player.is_playing())
            .unwrap_or(self.runtime_sfx_cursor);
        let _ = self.runtime_sfx[slot].play(cue);
        self.runtime_sfx_cursor = (slot + 1) % self.runtime_sfx.len();
    }

    /// Starts a package KACL one-shot after copying it into bounded owned storage.
    pub fn play_runtime_clip(&mut self, image: &[u8]) -> bool {
        self.play_runtime_clip_with_role(image, false)
    }

    /// Starts a package KACL clip on the background-music role.
    pub fn play_runtime_bgm_clip(&mut self, image: &[u8]) -> bool {
        self.play_runtime_clip_with_role(image, true)
    }

    fn play_runtime_clip_with_role(&mut self, image: &[u8], bgm: bool) -> bool {
        if image.len() > RUNTIME_CLIP_IMAGE_CAPACITY {
            self.runtime_stream = SimStreamingClip::new(image.to_vec(), self.sample_rate);
            self.runtime_clip.stop();
            self.runtime_clip_bgm = bgm;
            return self.runtime_stream.is_some();
        }
        let Ok(header) = ClipAssetHeader::decode(image) else {
            return false;
        };
        let mut limits = AudioLimits::v0_default();
        limits.sample_rate_hz = header.sample_rate_hz;
        self.runtime_stream = None;
        self.runtime_clip_bgm = bgm;
        self.runtime_clip.play_image(image, limits).is_ok()
    }

    /// Sets the sequence BGM/SFX bus gains (0..=256, 256 = unity). Lets the game
    /// integration rebalance the generated-sequence music against its effects.
    pub fn set_seq_volumes(&mut self, bgm: u16, sfx: u16) {
        if let Some(seq) = self.ensure_seq() {
            seq.set_bgm_volume(MixerVolume::new(bgm));
            seq.set_sfx_volume(MixerVolume::new(sfx));
        }
    }

    /// Test-only: the KotoAudio sequence bridge's counter snapshot, or `None` when
    /// the app never reached the primary sequence path (i.e. it stayed on the legacy
    /// MML/raw engines and `seq` was never created). Lets integration tests assert
    /// the KotoBlocks asset paths route to the primary bridge, not the legacy synth.
    #[cfg(test)]
    pub fn seq_counter(&self) -> Option<koto_audio::AudioCounterSnapshot> {
        self.seq.as_ref().map(|seq| seq.counter_snapshot())
    }

    /// Test/inspection hook for the SD-loaded owned KotoAudio BGM player.
    pub fn runtime_bgm_active(&self) -> bool {
        self.runtime_bgm.is_playing()
    }

    /// Inspection hook for owned or streamed package KACL playback.
    pub fn runtime_clip_active(&self) -> bool {
        self.runtime_clip.is_playing() || self.runtime_stream.is_some()
    }

    /// Test-only: whether the sequence bridge currently has an active BGM source.
    #[cfg(test)]
    pub fn seq_bgm_active(&self) -> bool {
        self.seq.as_ref().is_some_and(|seq| seq.is_bgm_active())
    }

    /// Set the BGM / SFX mix gains (clamped to `[0, 4]`). Lets a host or future
    /// `audio_set_volume` rebalance music against effects.
    pub fn set_gains(&mut self, bgm_gain: f64, sfx_gain: f64) {
        self.bgm_gain = bgm_gain.clamp(0.0, 4.0);
        self.sfx_gain = sfx_gain.clamp(0.0, 4.0);
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Set the output sample rate (window mode matches the audio device). Affects
    /// voices started afterwards; call before any `play_*`.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let rate = sample_rate.max(1);
        // The sequence bridge fixes its render rate at construction, so drop it on
        // a rate change; it rebuilds at the new rate on next use.
        if rate != self.sample_rate {
            self.seq = None;
            self.runtime_bgm = RuntimeCuePlayer::new(rate);
            self.runtime_sfx = [RuntimeCuePlayer::new(rate); RUNTIME_SFX_PLAYERS];
            self.runtime_clip = OwnedClipPlayer::new();
            self.runtime_stream = None;
            self.runtime_clip_bgm = false;
        }
        self.sample_rate = rate;
    }

    /// Silence everything: stop BGM, drop SFX voices, and discard queued raw PCM.
    /// Used when a new app is launched so it starts from a clean audio state.
    pub fn reset(&mut self) {
        self.stop_bgm();
        self.sfx.clear();
        self.raw.clear();
        // Drop the sequence bridge so a freshly launched app starts from silence;
        // it is recreated on first sequence use.
        self.seq = None;
        self.runtime_bgm.stop();
        for player in &mut self.runtime_sfx {
            player.stop();
        }
        self.runtime_clip.stop();
        self.runtime_stream = None;
        self.runtime_clip_bgm = false;
    }

    pub fn active_sfx(&self) -> usize {
        self.sfx.len()
    }

    /// Number of simultaneous BGM voices currently playing.
    pub fn active_bgm_voices(&self) -> usize {
        self.bgm.len()
    }

    /// Start (or restart) a looping BGM from app-supplied KotoMML text (built-in
    /// instruments only).
    pub fn play_bgm_mml(&mut self, mml: &str) -> Result<(), MmlError> {
        let tracks = parse_mml_multi_strict(mml, self.sample_rate)?;
        if tracks.is_empty() {
            self.stop_bgm();
            return Err(MmlError::UnknownCommand);
        }
        self.bgm = tracks
            .into_iter()
            .take(MAX_BGM_VOICES)
            .map(|track| MmlPlayer::new(track, self.sample_rate, true))
            .collect();
        Ok(())
    }

    pub fn stop_bgm(&mut self) {
        self.bgm.clear();
        self.runtime_bgm.stop();
        if self.runtime_clip_bgm {
            self.runtime_clip.stop();
            self.runtime_stream = None;
            self.runtime_clip_bgm = false;
        }
    }

    /// Trigger a one-shot SFX voice, dropping the oldest if the voice budget is full.
    pub fn play_sfx(&mut self, id: i32) {
        if let Some(track) = sfx_mml(id).and_then(|mml| parse_mml(mml, self.sample_rate).ok()) {
            if self.sfx.len() >= MAX_SFX_VOICES {
                self.sfx.remove(0);
            }
            self.sfx
                .push(MmlPlayer::new(track, self.sample_rate, false));
        }
    }

    pub fn play_sfx_mml(&mut self, mml: &str) -> Result<(), MmlError> {
        let track = parse_mml_strict(mml, self.sample_rate)?;
        if self.sfx.len() >= MAX_SFX_VOICES {
            self.sfx.remove(0);
        }
        self.sfx
            .push(MmlPlayer::new(track, self.sample_rate, false));
        Ok(())
    }

    /// Queue raw interleaved i16 PCM (little-endian bytes) for playback, downmixing
    /// stereo to mono. Returns the number of frames accepted.
    pub fn submit_pcm(&mut self, channels: i32, bytes: &[u8]) -> i32 {
        let channels = channels.max(1) as usize;
        let mut frames = 0;
        for frame in bytes.chunks_exact(2 * channels) {
            let mut sum = 0i32;
            for sample in frame.chunks_exact(2) {
                sum += i32::from(i16::from_le_bytes([sample[0], sample[1]]));
            }
            self.raw.push_back(clamp_i16(sum / channels as i32));
            frames += 1;
        }
        frames
    }

    /// Mix the next `out.len()` samples (mono) from BGM, SFX, and queued raw PCM,
    /// applying the BGM/SFX balance gains so effects stay audible over the music.
    pub fn render(&mut self, out: &mut [i16]) {
        for slot in out.iter_mut() {
            let mut mixed = 0.0f64;
            // Each BGM voice is scaled by the (sub-unity) BGM gain so a 3–4 voice
            // score (lead + bass + drum) stays within headroom before the i16 clamp.
            for voice in &mut self.bgm {
                mixed += voice.next_sample() as f64 * self.bgm_gain;
            }
            for voice in &mut self.sfx {
                mixed += voice.next_sample() as f64 * self.sfx_gain;
            }
            if let DecodeResult::Sample(sample) = self.runtime_bgm.next_sample() {
                mixed += sample as f64 * self.bgm_gain;
            }
            for player in &mut self.runtime_sfx {
                if let DecodeResult::Sample(sample) = player.next_sample() {
                    mixed += sample as f64 * self.sfx_gain;
                }
            }
            if let DecodeResult::Sample(sample) = self.runtime_clip.next_sample() {
                mixed += sample as f64
                    * if self.runtime_clip_bgm {
                        self.bgm_gain
                    } else {
                        self.sfx_gain
                    };
            }
            if let Some(stream) = &mut self.runtime_stream {
                if let DecodeResult::Sample(sample) = stream.next_sample() {
                    mixed += sample as f64
                        * if self.runtime_clip_bgm {
                            self.bgm_gain
                        } else {
                            self.sfx_gain
                        };
                }
            }
            if let Some(sample) = self.raw.pop_front() {
                mixed += sample as f64;
            }
            *slot = clamp_i16(mixed as i32);
        }
        self.sfx.retain(|voice| !voice.is_done());
        if self
            .runtime_stream
            .as_ref()
            .is_some_and(|stream| stream.decoder.is_finished() && !stream.looping)
        {
            self.runtime_stream = None;
        }
        // Fold the koto-audio generated-sequence bridge (KotoBlocks BGM/SFX) into
        // the same output stream, additively over the MML/raw mix above.
        if let Some(seq) = &mut self.seq {
            seq.mix_into(out);
        }
    }
}

/// Built-in KotoMML sound effects, keyed by `audio_id` SFX ids. Each cue picks a
/// host instrument (`@n`, see [`instrument`]) and runs through [`parse_mml`], so the
/// cues are chiptune-flavored rather than plain beeps: a thin pulse click to move, a
/// rising square arpeggio to rotate, a noise knock to lock, a bright ascending
/// arpeggio on a line clear, and a descending triangle on game over.
pub fn sfx_mml(id: i32) -> Option<&'static str> {
    use koto_core::runtime::audio_id;
    match id {
        // Shell UI cues stay short and restrained so repeated navigation is not tiring.
        audio_id::SFX_SHELL_NAV => Some("@5 T255 V5 O6 L32 C"),
        audio_id::SFX_SHELL_CONFIRM => Some("@1 T255 V7 O5 L32 E >C"),
        audio_id::SFX_SHELL_CANCEL => Some("@2 T220 V6 O5 L32 E <B"),
        _ => None,
    }
}

/// Encode mono i16 samples as a little-endian 16-bit PCM WAV byte stream.
pub fn wav_mono_bytes(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // channels = mono
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Write mono i16 samples as a little-endian 16-bit PCM WAV file (headless capture).
pub fn write_wav_mono(
    path: impl AsRef<Path>,
    sample_rate: u32,
    samples: &[i16],
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::File::create(path)?.write_all(&wav_mono_bytes(sample_rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_core::runtime::audio_id;

    #[test]
    fn parses_notes_rests_and_durations() {
        // C4 quarter, then an eighth rest, then E4 eighth at 120 BPM / 22050 Hz.
        let events = parse_mml("O4 L4 C R8 E8", 22_050).unwrap().events;
        assert_eq!(events.len(), 3);
        assert!(events[0].is_note);
        assert_eq!(events[0].midi_note, 60); // C4
        assert_eq!(events[0].samples, 11_025); // one quarter note
        assert!(!events[1].is_note);
        assert_eq!(events[1].samples, 5_512); // eighth rest (carry rounds .5)
        assert_eq!(events[2].midi_note, 64); // E4
    }

    #[test]
    fn handles_accidentals_and_octave_shifts() {
        let events = parse_mml("O4 L8 C# > C < < C", 22_050).unwrap().events;
        assert_eq!(events[0].midi_note, 61); // C#4
        assert_eq!(events[1].midi_note, 72); // C5 after '>'
        assert_eq!(events[2].midi_note, 48); // C3 after two '<'
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse_mml("L5 C", 22_050), Err(MmlError::BadLength));
        assert_eq!(parse_mml("O9 C", 22_050), Err(MmlError::OctaveRange));
        assert_eq!(parse_mml("X", 22_050), Err(MmlError::UnknownCommand));
    }

    #[test]
    fn instrument_select_tags_following_notes() {
        // Notes default to instrument 0 until an `@n` switch, which applies to every
        // following note until the next switch.
        let events = parse_mml("C @3 D @0 E", 22_050).unwrap().events;
        assert_eq!(events[0].instrument, 0);
        assert_eq!(events[1].instrument, 3);
        assert_eq!(events[2].instrument, 0);
    }

    #[test]
    fn parses_loop_markers() {
        // `[` marks the loop body start, `]` its end; the intro before `[` plays once.
        let track = parse_mml("C [ D E ]", 22_050).unwrap();
        assert_eq!(track.events.len(), 3);
        assert_eq!(track.loop_start, 1);
        assert_eq!(track.loop_end, 3);
        // No markers: the whole track is the loop body.
        let plain = parse_mml("C D", 22_050).unwrap();
        assert_eq!((plain.loop_start, plain.loop_end), (0, 2));
        // A `]` with no `[` is malformed.
        assert_eq!(parse_mml("C ]", 22_050), Err(MmlError::UnmatchedLoop));
    }

    #[test]
    fn looping_voice_repeats_only_the_body() {
        // Intro note then a one-note loop body. A looping voice must keep producing
        // sound long after the intro, and the intro never repeats.
        let track = parse_mml("@0 T200 V15 O5 L4 C [ E ]", 22_050).unwrap();
        assert_eq!(track.loop_start, 1);
        let mut player = MmlPlayer::new(track, 22_050, true);
        let buf: Vec<i16> = (0..22_050 * 2).map(|_| player.next_sample()).collect();
        assert!(!player.is_done(), "a looping voice never finishes");
        let tail = &buf[buf.len() - 4_096..];
        assert!(tail.iter().any(|&s| s != 0), "loop body fell silent");
    }

    #[test]
    fn strict_parse_rejects_unknown_instrument() {
        // Lenient parsing accepts any `@n` (the player falls back to `@0`); strict
        // parsing (tests / asset tooling) rejects ids outside the bank.
        assert!(parse_mml("@9 C", 22_050).is_ok());
        assert_eq!(
            parse_mml_strict("@9 C", 22_050),
            Err(MmlError::UnknownInstrument)
        );
        assert!(parse_mml_strict("@5 C", 22_050).is_ok());
    }

    #[test]
    fn bgm_gain_scales_the_music() {
        // A zero BGM gain silences the music with no SFX playing.
        let mut audio = SimAudio::new(22_050);
        audio.set_gains(0.0, 1.0);
        audio.play_bgm_mml("@0 T120 V10 O5 [ C D E F ]").unwrap();
        let mut buf = vec![0i16; 8_192];
        audio.render(&mut buf);
        assert!(
            buf.iter().all(|&s| s == 0),
            "zero BGM gain should be silent"
        );
    }

    #[test]
    fn instrument_bank_covers_each_waveform() {
        assert_eq!(instrument(0).wave, Waveform::Square);
        assert_eq!(instrument(1).wave, Waveform::Pulse25);
        assert_eq!(instrument(2).wave, Waveform::Triangle);
        assert_eq!(instrument(3).wave, Waveform::Saw);
        assert_eq!(instrument(4).wave, Waveform::Noise);
        assert_eq!(instrument(5).wave, Waveform::Pulse12);
        // Unknown ids fall back to the default lead rather than going silent.
        assert_eq!(instrument(99).wave, Waveform::Square);
    }

    #[test]
    fn noise_waveform_is_deterministic_but_varies() {
        // A noise instrument renders a non-constant, reproducible waveform (fixed LFSR
        // seed), unlike the periodic tone oscillators.
        let render = || {
            let track = parse_mml("@4 T120 V15 O4 L4 C", 22_050).unwrap();
            let mut player = MmlPlayer::new(track, 22_050, false);
            (0..2_000).map(|_| player.next_sample()).collect::<Vec<_>>()
        };
        let first = render();
        let second = render();
        assert_eq!(first, second, "noise must be deterministic for capture");
        let distinct = first
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert!(distinct > 2, "noise output looked like a plain tone");
    }

    #[test]
    fn all_builtin_tracks_parse() {
        // Strict parsing guards the bank against `@n` / length typos that lenient
        // playback would silently paper over.
        for id in [
            audio_id::SFX_SHELL_NAV,
            audio_id::SFX_SHELL_CONFIRM,
            audio_id::SFX_SHELL_CANCEL,
        ] {
            assert!(parse_mml_strict(sfx_mml(id).unwrap(), 22_050).is_ok());
        }
    }

    #[test]
    fn splits_score_into_independent_tracks() {
        // `#TRACK` markers separate voices; each keeps its own instrument/octave.
        let tracks = parse_mml_multi("#TRACK lead\n@0 O5 C\n#TRACK bass\n@2 O3 C", 22_050).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].events[0].instrument, 0);
        assert_eq!(tracks[0].events[0].midi_note, 72); // C5
        assert_eq!(tracks[1].events[0].instrument, 2);
        assert_eq!(tracks[1].events[0].midi_note, 48); // C3
                                                       // A plain `# comment` is not a track boundary, and markerless text is one track.
        let single = parse_mml_multi("# just a comment\nC D", 22_050).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].events.len(), 2);
    }

    #[test]
    fn multi_track_bgm_plays_all_voices() {
        let mut audio = SimAudio::new(22_050);
        audio
            .play_bgm_mml(
                "#TRACK lead\n@0 T150 V8 O5 L8 [ E G B G E G B G ]\n\
                 #TRACK bass\n@2 T150 V6 O3 L4 [ C C G G ]\n\
                 #TRACK drum\n@4 T150 V6 O2 L8 [ C R C R C C R C ]",
            )
            .unwrap();
        assert_eq!(audio.active_bgm_voices(), 3);
        // All voices loop, so sound persists well past the 4-beat body, and the mix
        // stays within the i16 range (no wrap from summing three voices).
        let mut buf = vec![0i16; 22_050 * 3];
        audio.render(&mut buf);
        let tail = &buf[buf.len() - 4_096..];
        assert!(tail.iter().any(|&s| s != 0), "multi-track BGM fell silent");
    }

    #[test]
    fn package_mml_starts_without_a_host_music_id() {
        let mut audio = SimAudio::new(22_050);
        audio
            .play_bgm_mml(
                "#TRACK lead\n@0 T120 O5 [ C4 D4 E4 F4 ]\n\
                 #TRACK bass\n@2 T120 O3 [ C2 G2 ]",
            )
            .unwrap();
        assert_eq!(audio.active_bgm_voices(), 2);
        let mut buf = vec![0i16; 4096];
        audio.render(&mut buf);
        assert!(buf.iter().any(|&sample| sample != 0));
    }

    #[test]
    fn bgm_renders_non_silent_and_loops() {
        let mut audio = SimAudio::new(22_050);
        audio.play_bgm_mml("@0 T120 V10 O5 [ C D E F ]").unwrap();
        // Render well past the end of the written track to prove it loops rather
        // than falling silent.
        let mut buf = vec![0i16; 22_050 * 4];
        audio.render(&mut buf);
        assert!(buf.iter().any(|&s| s != 0), "BGM produced only silence");
        let tail = &buf[buf.len() - 4_096..];
        assert!(tail.iter().any(|&s| s != 0), "BGM did not loop");
    }

    #[test]
    fn sfx_voices_are_bounded_and_finish() {
        let mut audio = SimAudio::new(22_050);
        for _ in 0..5 {
            audio.play_sfx(audio_id::SFX_SHELL_NAV);
        }
        assert!(audio.active_sfx() <= MAX_SFX_VOICES);
        // A one-shot SFX is finite: rendering a couple of seconds drains it.
        let mut buf = vec![0i16; 22_050 * 2];
        audio.render(&mut buf);
        assert_eq!(audio.active_sfx(), 0);
    }

    #[test]
    fn submitted_pcm_is_mixed_then_drained() {
        let mut audio = SimAudio::new(22_050);
        // Two mono frames: 100, -100.
        let bytes: Vec<u8> = [100i16, -100i16]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(audio.submit_pcm(1, &bytes), 2);
        let mut buf = [0i16; 3];
        audio.render(&mut buf);
        assert_eq!(buf, [100, -100, 0]);
    }

    #[test]
    fn codec_demo_pcm16_and_sld4_assets_are_both_audible() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/samples/audio_codecs/audio");
        for name in ["pcm16.kacl", "sld4.kacl"] {
            let image = std::fs::read(root.join(name)).expect("read demo KACL");
            let mut audio = SimAudio::new(16_000);
            assert!(audio.play_runtime_clip(&image), "{name} was rejected");
            let mut audible = false;
            let mut rendered = 0;
            while audio.runtime_clip_active() && rendered < 120_000 {
                let mut out = [0i16; 512];
                audio.render(&mut out);
                audible |= out.iter().any(|sample| sample.unsigned_abs() > 256);
                rendered += out.len();
            }
            assert!(audible, "{name} decoded as silence");
            assert!(rendered >= 109_714, "{name} ended early at {rendered}");
            assert!(!audio.runtime_clip_active(), "{name} did not finish");
        }
    }

    #[test]
    fn gallery_sld4_bgm_streams_and_loops_until_stopped() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/samples/full_color_tile_image/audio/music_sld4.kacl");
        let image = std::fs::read(path).expect("read gallery SLD4 KACL");
        assert!(image.len() > RUNTIME_CLIP_IMAGE_CAPACITY);

        let mut audio = SimAudio::new(16_000);
        assert!(audio.play_runtime_bgm_clip(&image));
        let mut audible_tail = false;
        for block in 0..1_100 {
            let mut out = [0i16; 512];
            audio.render(&mut out);
            if block > 1_070 {
                audible_tail |= out.iter().any(|sample| sample.unsigned_abs() > 256);
            }
        }
        assert!(audio.runtime_clip_active(), "looping BGM stopped at EOF");
        assert!(audible_tail, "looping BGM tail was silent");
        audio.stop_bgm();
        assert!(!audio.runtime_clip_active());
    }

    #[test]
    fn stereo_submit_downmixes_to_mono() {
        let mut audio = SimAudio::new(22_050);
        // One stereo frame (L=200, R=-100) downmixes to 50.
        let bytes: Vec<u8> = [200i16, -100i16]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(audio.submit_pcm(2, &bytes), 1);
        let mut buf = [0i16; 1];
        audio.render(&mut buf);
        assert_eq!(buf, [50]);
    }
}
