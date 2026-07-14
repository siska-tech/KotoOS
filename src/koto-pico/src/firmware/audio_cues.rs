//! Built-in hostcall cues for the Pico firmware.
//!
//! Package KMML does not live here: `app_runtime` reads it from SD, compiles a
//! pointer-free KotoAudio runtime image into PSRAM, and asks the CPU1 worker to
//! play an owned copy. This module only holds the fixed cues behind numeric
//! `play_sfx(id)` / `play_bgm(id)` hostcalls.

use koto_audio::{
    MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence, SequenceEvent,
    SequenceInstrument, SequenceWaveform, BUILTIN_INSTRUMENT_SQUARE, BUILTIN_INSTRUMENT_TRIANGLE,
    BUILTIN_SEQUENCE_INSTRUMENTS, SEQUENCE_REPEAT_INFINITE,
};

/// Returns the fixed blip cue behind the `play_sfx(id)` hostcall. The id set
/// mirrors the removed `tone_for_sfx_id` tone table (6/7/8 plus a default).
pub fn sfx_id_cue(id: i32) -> &'static Sequence<'static> {
    match id {
        6 => &BLIP_HIGH_SEQ,
        7 => &BLIP_TOP_SEQ,
        8 => &BLIP_LOW_SEQ,
        _ => &BLIP_MID_SEQ,
    }
}

/// Returns the built-in loop behind the `play_bgm(id)` hostcall, replacing the
/// removed built-in MML strings with equivalent authored sequences (id 0 plus a
/// default, as before).
pub fn builtin_bgm_cue(id: i32) -> &'static PolyphonicSequence<'static> {
    match id {
        0 => &BUILTIN_BGM_0,
        _ => &BUILTIN_BGM_1,
    }
}

// --- Hostcall blip cues -----------------------------------------------------

/// SFX tick rate: 64 ticks/second, so one tick is about 15.6 ms.
const SFX_TICK_RATE: u16 = 64;

/// Bright square lead for fixed hostcall blips.
static SFX_SQUARE: [SequenceInstrument; 1] = [SequenceInstrument::with_envelope(
    SequenceWaveform::Square,
    200,
    1,
    2,
)];
//
// Fixed one-shot square blips behind `play_sfx(id)`, replacing the removed tone
// path. Pitches and lengths approximate the old `tone_for_sfx_id` tones
// (frequency in hertz; the old durations of 70-100 ms round to 4-6 ticks).

static BLIP_HIGH_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 660,
        duration_ticks: 4,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_HIGH_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_HIGH_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_TOP_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 880,
        duration_ticks: 6,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_TOP_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_TOP_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_LOW_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 440,
        duration_ticks: 6,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_LOW_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_LOW_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_MID_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 720,
        duration_ticks: 4,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_MID_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_MID_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

// --- Built-in `play_bgm(id)` loops ------------------------------------------
//
// Authored equivalents of the removed built-in MML strings: a square lead over
// a triangle bass, one tick per eighth note (id 0 at T120 -> 4 ticks/s, the
// default at T150 -> 5 ticks/s), looping forever.

const fn bgm_note(pitch: u16, duration_ticks: u16, volume: u8, instrument_id: u8) -> SequenceEvent {
    SequenceEvent::Note {
        pitch,
        duration_ticks,
        volume,
        instrument_id,
    }
}

const BGM_LEAD_VOLUME: u8 = 136; // legacy V8
const BGM_BASS_VOLUME: u8 = 85; // legacy V5

static BUILTIN_BGM_0_LEAD_EVENTS: [SequenceEvent; 11] = [
    SequenceEvent::LoopStart,
    bgm_note(523, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // C5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(587, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // D5
    bgm_note(699, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // F5
    bgm_note(880, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // A5
    bgm_note(699, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // F5
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_0_BASS_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::LoopStart,
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    bgm_note(147, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // D3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_0_VOICES: [PolyphonicSequenceVoice<'static>; 2] = [
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_0_LEAD_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
        MixerVolume::UNITY,
    ),
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_0_BASS_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
        MixerVolume::UNITY,
    ),
];
static BUILTIN_BGM_0: PolyphonicSequence<'static> = PolyphonicSequence::new(&BUILTIN_BGM_0_VOICES);

static BUILTIN_BGM_1_LEAD_EVENTS: [SequenceEvent; 11] = [
    SequenceEvent::LoopStart,
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(988, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // B5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(988, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // B5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_1_BASS_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::LoopStart,
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_1_VOICES: [PolyphonicSequenceVoice<'static>; 2] = [
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_1_LEAD_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 5),
        MixerVolume::UNITY,
    ),
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_1_BASS_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 5),
        MixerVolume::UNITY,
    ),
];
static BUILTIN_BGM_1: PolyphonicSequence<'static> = PolyphonicSequence::new(&BUILTIN_BGM_1_VOICES);
