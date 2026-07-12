use koto_audio::{
    AudioLimits, AudioResult, Sequence, SequenceEvent, SequenceInstrument, SequencePitch,
    SequenceTempo, SEQUENCE_REPEAT_INFINITE,
};

const SHORT_BLIP: u8 = 0;
const SOFT_LOOP_LEAD: u8 = 1;

const SFX_TEMPO: SequenceTempo = SequenceTempo::from_bpm(180, 4);
const JINGLE_TEMPO: SequenceTempo = SequenceTempo::from_bpm(132, 4);
const LOOP_TEMPO: SequenceTempo = SequenceTempo::from_bpm(108, 4);

const INSTRUMENTS: [SequenceInstrument; 2] = [
    SequenceInstrument::short_blip(),
    SequenceInstrument::soft_triangle(),
];

const MOVE_BLIP_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::note(SequencePitch::C5, SFX_TEMPO.eighth(), 220, SHORT_BLIP),
    SequenceEvent::End,
];

const ROTATE_BLIP_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::note(
        SequencePitch::from_midi_note(76),
        SFX_TEMPO.eighth(),
        220,
        SHORT_BLIP,
    ),
    SequenceEvent::End,
];

const HARD_DROP_EVENTS: [SequenceEvent; 4] = [
    SequenceEvent::note(SequencePitch::from_midi_note(43), 1, 240, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::from_midi_note(36), 2, 220, SHORT_BLIP),
    SequenceEvent::rest(1),
    SequenceEvent::End,
];

const LINE_CLEAR_EVENTS: [SequenceEvent; 6] = [
    SequenceEvent::note(SequencePitch::C4, JINGLE_TEMPO.eighth(), 210, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::E4, JINGLE_TEMPO.eighth(), 210, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::G4, JINGLE_TEMPO.eighth(), 210, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::C5, JINGLE_TEMPO.quarter(), 220, SHORT_BLIP),
    SequenceEvent::rest(JINGLE_TEMPO.eighth()),
    SequenceEvent::End,
];

const GAME_OVER_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::note(SequencePitch::C5, JINGLE_TEMPO.quarter(), 210, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::G4, JINGLE_TEMPO.eighth(), 200, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::E4, JINGLE_TEMPO.eighth(), 190, SHORT_BLIP),
    SequenceEvent::note(SequencePitch::C4, JINGLE_TEMPO.quarter(), 180, SHORT_BLIP),
    SequenceEvent::note(
        SequencePitch::from_midi_note(48),
        JINGLE_TEMPO.quarter(),
        170,
        SHORT_BLIP,
    ),
    SequenceEvent::rest(JINGLE_TEMPO.eighth()),
    SequenceEvent::End,
];

const SIMPLE_LOOP_BGM_EVENTS: [SequenceEvent; 11] = [
    SequenceEvent::LoopStart,
    SequenceEvent::note(SequencePitch::C4, LOOP_TEMPO.eighth(), 160, SOFT_LOOP_LEAD),
    SequenceEvent::note(SequencePitch::E4, LOOP_TEMPO.eighth(), 150, SOFT_LOOP_LEAD),
    SequenceEvent::note(SequencePitch::G4, LOOP_TEMPO.eighth(), 160, SOFT_LOOP_LEAD),
    SequenceEvent::rest(LOOP_TEMPO.eighth()),
    SequenceEvent::note(SequencePitch::G4, LOOP_TEMPO.eighth(), 150, SOFT_LOOP_LEAD),
    SequenceEvent::note(SequencePitch::E4, LOOP_TEMPO.eighth(), 140, SOFT_LOOP_LEAD),
    SequenceEvent::note(SequencePitch::D4, LOOP_TEMPO.eighth(), 140, SOFT_LOOP_LEAD),
    SequenceEvent::rest(LOOP_TEMPO.eighth()),
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];

const MOVE_BLIP: Sequence<'static> =
    Sequence::with_tempo(&MOVE_BLIP_EVENTS, &INSTRUMENTS, SFX_TEMPO);
const ROTATE_BLIP: Sequence<'static> =
    Sequence::with_tempo(&ROTATE_BLIP_EVENTS, &INSTRUMENTS, SFX_TEMPO);
const HARD_DROP: Sequence<'static> =
    Sequence::with_tempo(&HARD_DROP_EVENTS, &INSTRUMENTS, SFX_TEMPO);
const LINE_CLEAR_JINGLE: Sequence<'static> =
    Sequence::with_tempo(&LINE_CLEAR_EVENTS, &INSTRUMENTS, JINGLE_TEMPO);
const GAME_OVER_JINGLE: Sequence<'static> =
    Sequence::with_tempo(&GAME_OVER_EVENTS, &INSTRUMENTS, JINGLE_TEMPO);
const SIMPLE_LOOP_BGM: Sequence<'static> =
    Sequence::with_tempo(&SIMPLE_LOOP_BGM_EVENTS, &INSTRUMENTS, LOOP_TEMPO);

const EXAMPLES: [(&str, Sequence<'static>); 6] = [
    ("move blip", MOVE_BLIP),
    ("rotate blip", ROTATE_BLIP),
    ("hard drop", HARD_DROP),
    ("line clear jingle", LINE_CLEAR_JINGLE),
    ("game over jingle", GAME_OVER_JINGLE),
    ("simple loop BGM", SIMPLE_LOOP_BGM),
];

fn main() -> AudioResult<()> {
    let limits = AudioLimits::default();

    for (name, sequence) in EXAMPLES {
        sequence.validate(limits)?;
        println!("{name}: {} events", sequence.events.len());
    }

    println!("simple loop BGM uses an infinite loop and is expected to be stopped by the caller");
    Ok(())
}
