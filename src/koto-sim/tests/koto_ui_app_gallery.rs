use std::path::PathBuf;

use koto_core::{BitmapFont, ConfigService, Locale, VmInputSnapshot, VmRunResult};
use koto_sim::{
    paint_app_session, parse_app_script, run_app_scenario, BytecodeAppSession, Framebuffer,
};

const APP_ID: &str = "dev.koto.samples.koto-ui-gallery";
const INTERACTION: &str =
    include_str!("../../../apps/samples/koto_ui_gallery/scenarios/interaction.txt");
const FONT: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");
const ENGLISH_GOLDEN: u64 = 0xaf4315bdff062100;
const MODAL_GOLDEN: u64 = 0x6af6b060ce4b7756;
const JAPANESE_MODAL_GOLDEN: u64 = 0xae798e6ad28c1461;
const PSEUDO_MODAL_GOLDEN: u64 = 0xab9e5669d02ae3ba;

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn frame_hash(session: &BytecodeAppSession) -> u64 {
    let font = BitmapFont::from_bytes(FONT).unwrap();
    let mut framebuffer = Framebuffer::new(320, 320);
    paint_app_session(&mut framebuffer.as_canvas(), &font, session);
    hash(framebuffer.as_canvas().pixels())
}

fn sdcard_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdcard_mock")
}

#[test]
fn app_gallery_script_exercises_semantic_ui_without_trapping() {
    let inputs = parse_app_script(INTERACTION).expect("valid Gallery scenario");
    let report = run_app_scenario(sdcard_root(), APP_ID, &inputs).expect("Gallery run");

    assert_eq!(report.frames, inputs.len());
    assert_eq!(report.result, VmRunResult::Yielded);
    // The TextField began as `Koto`; the script appends one multibyte character.
    assert_eq!(report.document, "Kotoあ");
    assert_eq!(report.budget.heap_budget, Some(24_576));
    assert!(report.budget.heap_bytes_peak <= report.budget.heap_request);
    assert!(report.budget.frame_fuel_peak <= report.budget.frame_fuel_cap);
    assert!(report.budget.host_calls_per_frame_peak <= 10);
    assert_eq!(
        report.budget.ui_session_sram_bytes,
        koto_core::UI_SESSION_SRAM_BYTES
    );
    assert!(report.budget.ui_session_sram_bytes <= 8_192);
    assert_eq!(report.budget.ui_render_commands_peak, 70);
    assert!(report.budget.frame_time_us_peak > 0);
    // KotoUI owns rendering; the bytecode never emits low-level draw calls.
    assert_eq!(report.budget.draw_rects_peak, 0);
    assert_eq!(report.budget.draw_pixels_peak, 0);
    assert_eq!(report.budget.text_draws_peak, 0);
}

#[test]
fn app_gallery_translations_are_text_assets_not_bytecode_rodata() {
    const BYTECODE: &[u8] =
        include_bytes!("../../../package_inputs/bytecode/sample_koto_ui_gallery.kbc");
    for embedded in [
        "KotoUI Gallery".as_bytes(),
        "KotoUI画廊".as_bytes(),
        "[[ KotoUI Gallery ]]".as_bytes(),
    ] {
        assert!(!BYTECODE
            .windows(embedded.len())
            .any(|window| window == embedded));
    }
    for locale in [
        include_str!("../../../apps/samples/koto_ui_gallery/locales/en-US.txt"),
        include_str!("../../../apps/samples/koto_ui_gallery/locales/ja-JP.txt"),
        include_str!("../../../apps/samples/koto_ui_gallery/locales/qps-ploc.txt"),
    ] {
        let lines: Vec<_> = locale.lines().collect();
        assert_eq!(lines.len(), 22);
        assert!(lines.iter().all(|line| !line.is_empty()));
    }
}

#[test]
fn app_gallery_text_capacity_rejects_recovers_and_keeps_memory_stable() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }

    let mut down = VmInputSnapshot::empty();
    down.intent_bits = koto_core::runtime::text_intent::DOWN;
    let mut right = VmInputSnapshot::empty();
    right.intent_bits = koto_core::runtime::text_intent::RIGHT;
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(right).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(6));
    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);

    let mut text = VmInputSnapshot::empty();
    text.text_codepoint = 'x' as u32;
    for _ in 0..60 {
        assert_eq!(session.step_frame(text).unwrap(), VmRunResult::Yielded);
    }
    assert_eq!(session.retained_ui_value(6).unwrap().len(), 64);
    let heap_at_limit = session.budget().heap_bytes_peak;

    assert_eq!(session.step_frame(text).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(6).unwrap().len(), 64);
    assert_eq!(session.retained_ui_text(2), Some("Capacity error"));
    assert_eq!(session.budget().heap_bytes_peak, heap_at_limit);

    let mut backspace = VmInputSnapshot::empty();
    backspace.intent_bits = koto_core::runtime::text_intent::BACKSPACE;
    assert_eq!(session.step_frame(backspace).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(6).unwrap().len(), 63);
    assert_eq!(session.step_frame(text).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(6).unwrap().len(), 64);
    assert_eq!(session.budget().heap_bytes_peak, heap_at_limit);
}

#[test]
fn app_gallery_renders_and_cancels_ime_composition_without_editing_value() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }

    let mut down = VmInputSnapshot::empty();
    down.intent_bits = koto_core::runtime::text_intent::DOWN;
    let mut right = VmInputSnapshot::empty();
    right.intent_bits = koto_core::runtime::text_intent::RIGHT;
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(right).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(6));
    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);

    let mut toggle = VmInputSnapshot::empty();
    toggle.intent_bits = koto_core::runtime::text_intent::IME_TOGGLE;
    assert_eq!(session.step_frame(toggle).unwrap(), VmRunResult::Yielded);

    let mut text = VmInputSnapshot::empty();
    text.text_codepoint = 'k' as u32;
    assert_eq!(session.step_frame(text).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ime_line().mode, koto_core::MemoImeMode::Composing);
    assert_eq!(session.ime_line().pending_romaji, "k");
    assert_eq!(session.retained_ui_value(6), Some("Koto"));
    assert!(session.ui_text().iter().any(|(_, _, text)| text == "k"));

    let mut cancel = VmInputSnapshot::empty();
    cancel.intent_bits = koto_core::runtime::text_intent::CANCEL;
    assert_eq!(session.step_frame(cancel).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ime_line().mode, koto_core::MemoImeMode::Empty);
    assert_eq!(session.retained_ui_value(6), Some("Koto"));
    assert!(!session.ui_text().iter().any(|(_, _, text)| text == "k"));
}

#[test]
fn app_gallery_skk_candidate_replaces_reading_and_uses_standard_commit_cancel_keys() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }

    let mut down = VmInputSnapshot::empty();
    down.intent_bits = koto_core::runtime::text_intent::DOWN;
    let mut right = VmInputSnapshot::empty();
    right.intent_bits = koto_core::runtime::text_intent::RIGHT;
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(right).unwrap(), VmRunResult::Yielded);

    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);

    let mut toggle = VmInputSnapshot::empty();
    toggle.intent_bits = koto_core::runtime::text_intent::IME_TOGGLE;
    assert_eq!(session.step_frame(toggle).unwrap(), VmRunResult::Yielded);

    let mut shift = VmInputSnapshot::empty();
    shift.intent_bits = koto_core::runtime::text_intent::SHIFT;
    assert_eq!(session.step_frame(shift).unwrap(), VmRunResult::Yielded);
    for ch in ['k', 'a', 's', 'a'] {
        let mut input = VmInputSnapshot::empty();
        input.text_codepoint = ch as u32;
        assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
    }
    assert_eq!(session.retained_ui_value(6), Some("Koto"));
    assert!(session.ui_text().iter().any(|(_, _, text)| text == "かさ"));

    // A physical Space carries both its codepoint and CONVERT. During an
    // active SKK reading the host consumes both as candidate conversion.
    let mut space = VmInputSnapshot::empty();
    space.text_codepoint = ' ' as u32;
    space.intent_bits = koto_core::runtime::text_intent::CONVERT;
    assert_eq!(session.step_frame(space).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ime_line().candidate, Some("傘"));
    assert!(session.ui_text().iter().any(|(_, _, text)| text == "傘"));
    assert!(!session.ui_text().iter().any(|(_, _, text)| text == "かさ"));

    // Enter commits the candidate without submitting the TextField.
    let mut enter = VmInputSnapshot::empty();
    enter.intent_bits = koto_core::runtime::text_intent::NEWLINE;
    enter.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(enter).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(6), Some("Koto傘"));
    assert_eq!(session.ime_line().mode, koto_core::MemoImeMode::Empty);
    assert_ne!(session.retained_ui_text(2), Some("Text submitted"));

    assert_eq!(session.step_frame(shift).unwrap(), VmRunResult::Yielded);
    for ch in ['k', 'a', 's', 'a'] {
        let mut input = VmInputSnapshot::empty();
        input.text_codepoint = ch as u32;
        assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
    }
    assert_eq!(session.step_frame(space).unwrap(), VmRunResult::Yielded);
    // Ctrl+G carries CANCEL and may still have a typed `g` in the host
    // snapshot; cancellation must consume it instead of appending it.
    let mut ctrl_g = VmInputSnapshot::empty();
    ctrl_g.intent_bits = koto_core::runtime::text_intent::CANCEL;
    ctrl_g.text_codepoint = 'g' as u32;
    assert_eq!(session.step_frame(ctrl_g).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(6), Some("Koto傘"));
    assert_eq!(session.ime_line().mode, koto_core::MemoImeMode::Empty);
}

#[test]
fn app_gallery_event_queue_overflow_reports_then_recovers_in_order() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    let heap_before = session.budget().heap_bytes_peak;
    for index in 0..10 {
        let mut input = VmInputSnapshot::empty();
        input.intent_bits = if index % 2 == 0 {
            koto_core::runtime::text_intent::RIGHT
        } else {
            koto_core::runtime::text_intent::LEFT
        };
        session.inject_ui_input_without_vm(input);
    }

    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    let events = session.ui_polled_events();
    assert_eq!(events.len(), koto_core::UI_EVENT_QUEUE_CAPACITY + 1);
    assert!(events[..koto_core::UI_EVENT_QUEUE_CAPACITY]
        .iter()
        .all(|event| event.0 == 8));
    assert_eq!(events[koto_core::UI_EVENT_QUEUE_CAPACITY], (9, 0, 0, 2));
    assert_eq!(session.retained_ui_text(2), Some("Capacity error"));
    assert_eq!(session.budget().heap_bytes_peak, heap_before);

    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert!(session.ui_polled_events().is_empty());
    assert_eq!(session.budget().heap_bytes_peak, heap_before);
}

#[test]
fn app_gallery_script_can_exit_after_editing() {
    let mut script = String::from(INTERACTION);
    // First Cancel leaves TextField editing; the second reaches the app.
    script.push_str("\ncancel\ncancel\n");
    let inputs = parse_app_script(&script).unwrap();
    let report = run_app_scenario(sdcard_root(), APP_ID, &inputs).unwrap();
    assert_eq!(report.result, VmRunResult::Exited(0));
    assert_eq!(report.document, "Kotoあ");
}

#[test]
fn app_gallery_f10_intent_exits_without_a_ui_cancel_event() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).expect("Gallery launch");
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session
            .step_frame(VmInputSnapshot {
                intent_bits: koto_core::runtime::text_intent::EXIT,
                ..VmInputSnapshot::empty()
            })
            .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn app_gallery_applies_live_english_japanese_and_pseudolocale_cycle() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).expect("Gallery launch");
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(1), Some("KotoUI Gallery"));
    assert_eq!(session.retained_ui_text(3), Some("Open dialog"));
    // The first frame after mount establishes the host generation observed by
    // the retained session; later changes enqueue LocaleChanged.
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );

    let mut config = ConfigService::new();
    assert!(config.set_locale(Locale::JaJp));
    session.set_config_snapshot(config.snapshot());
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(1), Some("KotoUI画廊"));
    assert_eq!(session.retained_ui_text(2), Some("全UI部品"));
    assert_eq!(session.retained_ui_text(3), Some("開く"));
    assert_eq!(session.retained_ui_text(5), Some(""));
    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_text(2), Some("対話を開く"));

    // The deterministic ASCII pseudolocale expands every single-line label.
    assert!(config.set_locale(Locale::QpsPloc));
    session.set_config_snapshot(config.snapshot());
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(1), Some("[[ KotoUI Gallery ]]"));
    assert_eq!(session.retained_ui_text(2), Some("[ All v1 components + ]"));
    assert_eq!(session.retained_ui_text(3), Some("[ Open dialog! ]"));
    for (widget, english, pseudo) in [
        (1, "KotoUI Gallery", "[[ KotoUI Gallery ]]"),
        (2, "All v1 components", "[ All v1 components + ]"),
        (3, "Open dialog", "[ Open dialog! ]"),
        (4, "Enable editing", "[ Enable editing!! ]"),
        (6, "Text", "[Text]"),
        (7, "Disabled", "[ Disabled]"),
        (8, "Confirm", "[Confirm!]"),
        (9, "Close", "[Close]"),
        (10, "Modal focus", "[ Modal focus! ]"),
    ] {
        assert_eq!(session.retained_ui_text(widget), Some(pseudo));
        let expansion = pseudo.len() * 100 / english.len();
        assert!((135..=150).contains(&expansion));
    }
    // Focus remains trapped in the open Dialog across the locale update; its
    // action closes the modal and selects the pseudolocalized status message.
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_text(2), Some("[ Dialog closed! ]"));

    assert!(config.set_locale(Locale::EnUs));
    session.set_config_snapshot(config.snapshot());
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(1), Some("KotoUI Gallery"));
    assert_eq!(session.retained_ui_text(2), Some("All v1 components"));
    assert_eq!(session.retained_ui_text(3), Some("Open dialog"));
}

#[test]
fn app_gallery_frames_match_320_square_locale_and_modal_goldens() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    let english = frame_hash(&session);

    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);
    assert!(session.ui_text().iter().any(|(_, _, text)| text == "Close"));
    assert!(session
        .ui_text()
        .iter()
        .any(|(_, _, text)| text == "Modal focus"));
    let modal = frame_hash(&session);

    let mut config = ConfigService::new();
    assert!(config.set_locale(Locale::JaJp));
    session.set_config_snapshot(config.snapshot());
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    let japanese_modal = frame_hash(&session);

    assert!(config.set_locale(Locale::QpsPloc));
    session.set_config_snapshot(config.snapshot());
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    let pseudo_modal = frame_hash(&session);

    println!(
        "app gallery en={english:#018x} modal={modal:#018x} ja={japanese_modal:#018x} qps={pseudo_modal:#018x}"
    );
    assert_eq!(english, ENGLISH_GOLDEN);
    assert_eq!(modal, MODAL_GOLDEN);
    assert_eq!(japanese_modal, JAPANESE_MODAL_GOLDEN);
    assert_eq!(pseudo_modal, PSEUDO_MODAL_GOLDEN);
}

#[test]
fn app_gallery_trace_pins_focus_responses_damage_and_idle() {
    let mut session = BytecodeAppSession::launch(sdcard_root(), APP_ID).unwrap();
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_focused_id(), Some(3));
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(session.ui_presented_damage(), &[(0, 0, 320, 320)]);

    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.ui_present_calls_this_frame(), 0);
    assert_eq!(session.ui_paint_count_this_frame(), 0);
    assert!(session.ui_presented_damage().is_empty());
    assert!(session.ui_polled_events().is_empty());

    let mut activate = VmInputSnapshot::empty();
    activate.pressed_bits = 1 << 4;
    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(9));
    assert_eq!(session.ui_polled_events(), &[(1, 3, 0, 0)]);
    assert_eq!(session.ui_present_calls_this_frame(), 2);
    assert_eq!(session.ui_paint_count_this_frame(), 2);
    assert_eq!(
        session.ui_presented_damage(),
        &[(12, 52, 278, 190), (12, 28, 296, 20)]
    );

    let mut cancel = VmInputSnapshot::empty();
    cancel.intent_bits = koto_core::runtime::text_intent::CANCEL;
    assert_eq!(session.step_frame(cancel).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(3));
    assert_eq!(session.ui_polled_events(), &[(7, 9, 0, 0)]);
    assert_eq!(session.ui_present_calls_this_frame(), 2);
    assert_eq!(session.ui_paint_count_this_frame(), 2);
    assert_eq!(
        session.ui_presented_damage(),
        &[(12, 52, 278, 190), (12, 28, 296, 20)]
    );

    let mut right = VmInputSnapshot::empty();
    right.intent_bits = koto_core::runtime::text_intent::RIGHT;
    assert_eq!(session.step_frame(right).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(4));
    assert_eq!(session.ui_polled_events(), &[(8, 4, 3, 0)]);
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(
        session.ui_presented_damage(),
        &[(12, 52, 142, 28), (166, 52, 142, 28)]
    );

    assert_eq!(session.step_frame(activate).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ui_polled_events(), &[(2, 4, 1, 0)]);

    let mut left = VmInputSnapshot::empty();
    left.intent_bits = koto_core::runtime::text_intent::LEFT;
    assert_eq!(session.step_frame(left).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(3));
    assert_eq!(session.ui_polled_events(), &[(8, 3, 4, 0)]);

    let mut down = VmInputSnapshot::empty();
    down.intent_bits = koto_core::runtime::text_intent::DOWN;
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(5));
    assert_eq!(session.ui_polled_events(), &[(8, 5, 3, 0)]);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ui_polled_events(), &[(4, 5, 1, 0)]);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.ui_polled_events(), &[(4, 5, 2, 0)]);
    let painted: Vec<_> = session
        .ui_text()
        .iter()
        .map(|(_, _, text)| text.as_str())
        .collect();
    assert!(!painted.contains(&"Label"));
    assert!(painted.contains(&"Button"));
    assert!(painted.contains(&"Text field"));
}
