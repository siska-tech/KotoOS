//! Adapters from KotoOS input snapshots to device-independent KotoUI events.
//!
//! Existing snapshots represent both an initial press and an OS-generated key
//! repeat as a pulse in `pressed`/`intent_bits`. Callers that can distinguish
//! repeats pass the corresponding repeat mask so KotoUI retains that phase.

use koto_ui::{EventBuffer, EventBufferFull, EventPhase, Navigation, UiAction, UiEvent};

use crate::{runtime::text_intent, Buttons, InputState, VmInputSnapshot};

pub fn push_input_state<const N: usize>(
    input: &InputState,
    repeated: &Buttons,
    output: &mut EventBuffer<N>,
) -> Result<(), EventBufferFull> {
    let pressed = &input.pressed;
    push_button(
        pressed.up,
        repeated.up,
        input.released.up,
        UiAction::Navigate(Navigation::Up),
        output,
    )?;
    push_button(
        pressed.down,
        repeated.down,
        input.released.down,
        UiAction::Navigate(Navigation::Down),
        output,
    )?;
    push_button(
        pressed.left,
        repeated.left,
        input.released.left,
        UiAction::Navigate(Navigation::Left),
        output,
    )?;
    push_button(
        pressed.right,
        repeated.right,
        input.released.right,
        UiAction::Navigate(Navigation::Right),
        output,
    )?;
    push_button(
        pressed.confirm,
        repeated.confirm,
        input.released.confirm,
        UiAction::Activate,
        output,
    )?;
    push_button(
        pressed.cancel,
        repeated.cancel,
        input.released.cancel,
        UiAction::Cancel,
        output,
    )?;
    if let Some(character) = input.unicode_codepoint {
        output.push(UiEvent::pressed(UiAction::Text(character)))?;
    }
    Ok(())
}

pub fn push_vm_input<const N: usize>(
    input: VmInputSnapshot,
    repeated_intents: u32,
    output: &mut EventBuffer<N>,
) -> Result<(), EventBufferFull> {
    let intents = input.intent_bits;
    push_intent(
        intents,
        repeated_intents,
        text_intent::UP,
        UiAction::Navigate(Navigation::Up),
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::DOWN,
        UiAction::Navigate(Navigation::Down),
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::LEFT,
        UiAction::Navigate(Navigation::Left),
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::RIGHT,
        UiAction::Navigate(Navigation::Right),
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::BACKSPACE,
        UiAction::Backspace,
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::DELETE,
        UiAction::Delete,
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::HOME,
        UiAction::Home,
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::END,
        UiAction::End,
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::NEWLINE,
        UiAction::Submit,
        output,
    )?;
    push_intent(
        intents,
        repeated_intents,
        text_intent::CANCEL,
        UiAction::Cancel,
        output,
    )?;
    if let Some(character) =
        char::from_u32(input.text_codepoint).filter(|character| *character != '\0')
    {
        output.push(UiEvent::pressed(UiAction::Text(character)))?;
    }
    Ok(())
}

fn push_button<const N: usize>(
    pressed: bool,
    repeated: bool,
    released: bool,
    action: UiAction,
    output: &mut EventBuffer<N>,
) -> Result<(), EventBufferFull> {
    if pressed {
        output.push(UiEvent {
            action,
            phase: if repeated {
                EventPhase::Repeated
            } else {
                EventPhase::Pressed
            },
        })?;
    }
    if released {
        output.push(UiEvent::released(action))?;
    }
    Ok(())
}

fn push_intent<const N: usize>(
    intents: u32,
    repeated_intents: u32,
    flag: u32,
    action: UiAction,
    output: &mut EventBuffer<N>,
) -> Result<(), EventBufferFull> {
    push_button(
        intents & flag != 0,
        repeated_intents & flag != 0,
        false,
        action,
        output,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_snapshot_preserves_repeat_phase_and_unicode() {
        let input = InputState {
            pressed: Buttons {
                down: true,
                confirm: true,
                ..Buttons::default()
            },
            unicode_codepoint: Some('日'),
            ..InputState::default()
        };
        let repeated = Buttons {
            down: true,
            ..Buttons::default()
        };
        let mut events = EventBuffer::<4>::new();
        push_input_state(&input, &repeated, &mut events).unwrap();
        assert_eq!(
            events.iter().collect::<std::vec::Vec<_>>(),
            [
                UiEvent::repeated(UiAction::Navigate(Navigation::Down)),
                UiEvent::pressed(UiAction::Activate),
                UiEvent::pressed(UiAction::Text('日')),
            ]
        );
    }

    #[test]
    fn native_snapshot_emits_button_release_phase() {
        let input = InputState {
            released: Buttons {
                confirm: true,
                ..Buttons::default()
            },
            ..InputState::default()
        };
        let mut events = EventBuffer::<1>::new();
        push_input_state(&input, &Buttons::default(), &mut events).unwrap();
        assert_eq!(
            events.iter().collect::<std::vec::Vec<_>>(),
            [UiEvent::released(UiAction::Activate)]
        );
    }

    #[test]
    fn vm_intents_are_ordered_and_preserve_repeat_phase() {
        let input = VmInputSnapshot {
            text_codepoint: 'あ' as u32,
            intent_bits: text_intent::LEFT | text_intent::BACKSPACE | text_intent::NEWLINE,
            ..VmInputSnapshot::empty()
        };
        let mut events = EventBuffer::<4>::new();
        push_vm_input(
            input,
            text_intent::LEFT | text_intent::BACKSPACE,
            &mut events,
        )
        .unwrap();
        assert_eq!(
            events.iter().collect::<std::vec::Vec<_>>(),
            [
                UiEvent::repeated(UiAction::Navigate(Navigation::Left)),
                UiEvent::repeated(UiAction::Backspace),
                UiEvent::pressed(UiAction::Submit),
                UiEvent::pressed(UiAction::Text('あ')),
            ]
        );
    }

    #[test]
    fn invalid_unicode_and_ime_only_intents_are_not_ui_events() {
        let input = VmInputSnapshot {
            text_codepoint: u32::MAX,
            intent_bits: text_intent::SHIFT | text_intent::CONVERT | text_intent::COMMIT,
            ..VmInputSnapshot::empty()
        };
        let mut events = EventBuffer::<1>::new();
        push_vm_input(input, 0, &mut events).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn adapter_reports_fixed_buffer_capacity() {
        let input = InputState {
            pressed: Buttons {
                up: true,
                down: true,
                ..Buttons::default()
            },
            ..InputState::default()
        };
        assert_eq!(
            push_input_state(&input, &Buttons::default(), &mut EventBuffer::<1>::new()),
            Err(EventBufferFull)
        );
    }
}
