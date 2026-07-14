//! Shared KotoMML -> KotoAudio `SequenceEvent` conversion.
//!
//! This is the conversion half of the cue-table generator (see `main.rs`),
//! split out so the `.kmml` audition CLI (`koto-mml`, KOTO-0188) renders
//! through exactly the same native KotoAudio conversion as the generated Pico
//! cue tables — what audition plays is what the device plays.

use koto_audio::{
    CompactEvent, SequenceEvent, BUILTIN_INSTRUMENT_SQUARE, SEQUENCE_REPEAT_INFINITE,
};
use koto_audio_tools::mml::{parse_mml_to_compact_sequence_table_with_options, MmlParseOptions};
use koto_audio_tools::CompactSequenceTable;

/// One converted track: adapted events plus voice gain.
pub struct ConvertedTrack {
    pub events: Vec<SequenceEvent>,
    pub gain: u16,
}

/// A parsed and converted score, ready to build runtime sequences from.
pub struct ConvertedScore {
    pub tick_rate_hz: u16,
    pub tracks: Vec<ConvertedTrack>,
}

/// Parse `.kmml` text and convert it to KotoAudio sequence tracks.
///
/// With `loop_forever` the BGM rule is applied: every track is wrapped (or its
/// finite loop regions forced) to [`SEQUENCE_REPEAT_INFINITE`], like the
/// runtime BGM playback loops. Without it the score keeps its authored
/// loop shape (a `[ ]` region still loops as written).
pub fn convert_mml_text(text: &str, loop_forever: bool) -> Result<ConvertedScore, String> {
    let table = parse_mml_to_compact_sequence_table_with_options(text, MmlParseOptions::default())
        .map_err(|error| format!("parse failed: {error:?}"))?;
    let mut tracks = convert_tracks(&table);
    if loop_forever {
        for track in &mut tracks {
            ensure_infinite_loop(&mut track.events);
        }
    }
    Ok(ConvertedScore {
        tick_rate_hz: table.tempo.tick_rate_hz,
        tracks,
    })
}

/// Adapts native KotoAudio compact tracks to `SequenceEvent`s against the
/// builtin instrument table. This mirrors
/// `CompactTrack::adapt_to_sequence` (instrument volume is 255 in every table
/// the MML frontend emits, so note volumes pass through unscaled).
pub fn convert_tracks(table: &CompactSequenceTable) -> Vec<ConvertedTrack> {
    table
        .tracks
        .iter()
        .map(|track| ConvertedTrack {
            gain: track.gain.get(),
            events: track
                .events
                .iter()
                .map(|event| convert_event(*event, table))
                .collect(),
        })
        .collect()
}

fn convert_event(event: CompactEvent, table: &CompactSequenceTable) -> SequenceEvent {
    match event {
        CompactEvent::Note {
            pitch,
            duration_ticks,
            volume,
            instrument_id,
        } => {
            let builtin_id = table
                .instruments
                .get(usize::from(instrument_id))
                .map_or(BUILTIN_INSTRUMENT_SQUARE, |instrument| {
                    instrument.builtin_id
                });
            SequenceEvent::Note {
                pitch,
                duration_ticks,
                volume,
                instrument_id: builtin_id,
            }
        }
        CompactEvent::Rest { duration_ticks } => SequenceEvent::Rest { duration_ticks },
        CompactEvent::LoopStart => SequenceEvent::LoopStart,
        CompactEvent::LoopEnd { repeat_count } => SequenceEvent::LoopEnd { repeat_count },
        CompactEvent::End => SequenceEvent::End,
    }
}

/// BGM must loop forever even without `[ ]`:
/// wrap the whole track when the source has no loop region, and force finite
/// loop regions to infinite.
pub fn ensure_infinite_loop(events: &mut Vec<SequenceEvent>) {
    let mut has_loop = false;
    for event in events.iter_mut() {
        if let SequenceEvent::LoopEnd { repeat_count } = event {
            *repeat_count = SEQUENCE_REPEAT_INFINITE;
            has_loop = true;
        }
    }
    if !has_loop {
        let end = events
            .iter()
            .position(|event| matches!(event, SequenceEvent::End))
            .unwrap_or(events.len());
        events.insert(
            end,
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
        );
        events.insert(0, SequenceEvent::LoopStart);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_loop_region_is_wrapped_infinite() {
        let mut events = vec![
            SequenceEvent::Note {
                pitch: 440,
                duration_ticks: 4,
                volume: 200,
                instrument_id: 3,
            },
            SequenceEvent::End,
        ];
        ensure_infinite_loop(&mut events);
        assert!(matches!(events[0], SequenceEvent::LoopStart));
        assert!(matches!(
            events[2],
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE
            }
        ));
        assert!(matches!(events[3], SequenceEvent::End));
    }

    #[test]
    fn finite_loop_regions_are_forced_infinite() {
        let mut events = vec![
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 4 },
            SequenceEvent::LoopEnd { repeat_count: 3 },
            SequenceEvent::End,
        ];
        ensure_infinite_loop(&mut events);
        assert!(matches!(
            events[2],
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE
            }
        ));
    }

    /// `convert_mml_text` end to end: native values are preserved, loop wrap
    /// is applied on request, and authored loop shape is preserved without it.
    #[test]
    fn convert_mml_text_applies_native_values_and_loop_policy() {
        let text = "@3 T120 V85 O4 L4\nC D E";
        let once = convert_mml_text(text, false).expect("converts");
        assert_eq!(once.tracks.len(), 1);
        assert!(once.tracks[0]
            .events
            .iter()
            .all(|event| !matches!(event, SequenceEvent::LoopStart)));
        assert!(once.tracks[0].events.iter().any(|event| matches!(
            event,
            SequenceEvent::Note {
                volume: 85,
                instrument_id: 3,
                ..
            }
        )));

        let looped = convert_mml_text(text, true).expect("converts");
        assert!(matches!(
            looped.tracks[0].events[0],
            SequenceEvent::LoopStart
        ));
    }
}
