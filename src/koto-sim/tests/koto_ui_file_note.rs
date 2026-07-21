//! KOTO-0221: the File Note pilot — the first existing app migrated from
//! per-frame immediate drawing to the app-facing KotoUI ABI. These tests pin
//! the preserved sandbox behavior (first-run creation, read-back, exact save
//! bytes, persistence across relaunch), the semantic form behavior (focus,
//! disabled Save, activation, capacity, cancellation, exit), localization,
//! damage/idle traces, and the 320x320 goldens.

use std::fs;
use std::path::PathBuf;

use koto_core::{BitmapFont, ConfigService, Locale, VmInputSnapshot, VmRunResult};
use koto_sim::{
    paint_app_session, parse_app_script, run_app_scenario, BytecodeAppSession, Framebuffer,
};

const APP_ID: &str = "dev.koto.samples.file-note";
const PACKAGE: &[u8] = include_bytes!("../../../sdcard_mock/apps/sample_file_note.kpa");
const INTERACTION: &str = include_str!("../../../apps/samples/file_note/scenarios/interaction.txt");
const FONT: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");
const DEFAULT_NOTE: &str = "saved from SDK sample";

const PANEL: u16 = 1;
const STATUS: u16 = 2;
const FIELD: u16 = 3;
const SAVE: u16 = 4;
const RELOAD: u16 = 5;

const ENGLISH_GOLDEN: u64 = 0x262d0df2cc9456ab;
const JAPANESE_GOLDEN: u64 = 0xd0ea4202a69cd848;
const PSEUDO_GOLDEN: u64 = 0x3142fa94bad190d4;

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

/// A writable per-test SD root holding only the committed File Note package.
/// The gallery suite runs read-only from `sdcard_mock`; File Note writes into
/// its save sandbox, so every test isolates `data/` in its own root.
fn test_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("koto_sim_file_note_{name}"));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("apps")).unwrap();
    fs::write(root.join("apps").join("sample_file_note.kpa"), PACKAGE).unwrap();
    root
}

fn note_path(root: &std::path::Path) -> PathBuf {
    root.join("data").join(APP_ID).join("note.txt")
}

fn seed_note(root: &std::path::Path, bytes: &[u8]) {
    fs::create_dir_all(note_path(root).parent().unwrap()).unwrap();
    fs::write(note_path(root), bytes).unwrap();
}

/// Locale asset, sandbox round trip, and mount+present each take one
/// deterministic startup frame.
fn boot(session: &mut BytecodeAppSession) {
    for _ in 0..3 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
}

fn intent(bits: u32) -> VmInputSnapshot {
    VmInputSnapshot {
        intent_bits: bits,
        ..VmInputSnapshot::empty()
    }
}

fn activate() -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.pressed_bits = 1 << 4;
    input
}

fn typed(ch: char) -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.text_codepoint = ch as u32;
    input
}

#[test]
fn first_run_creates_the_note_and_mounts_the_form() {
    let root = test_root("first_run");
    assert!(!note_path(&root).exists());
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    // The preserved first-run behavior: the note now exists with the original
    // sample bytes, the field shows the read-back, and the status reports it.
    assert_eq!(fs::read(note_path(&root)).unwrap(), DEFAULT_NOTE.as_bytes());
    assert_eq!(session.retained_ui_text(STATUS), Some("Note created"));
    assert_eq!(session.retained_ui_value(FIELD), Some(DEFAULT_NOTE));
    assert_eq!(session.retained_ui_text(PANEL), Some("File Note"));
    assert_eq!(session.retained_ui_text(SAVE), Some("Save"));
    assert_eq!(session.retained_ui_text(RELOAD), Some("Reload"));
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));

    // KotoUI owns component pixels; the bytecode never draws.
    assert_eq!(session.budget().draw_rects_peak, 0);
    assert_eq!(session.budget().draw_pixels_peak, 0);
    assert_eq!(session.budget().text_draws_peak, 0);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_note_loads_without_rewriting_the_file() {
    let root = test_root("existing_note");
    seed_note(&root, "hello ノート".as_bytes());
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    assert_eq!(session.retained_ui_text(STATUS), Some("Note loaded"));
    assert_eq!(session.retained_ui_value(FIELD), Some("hello ノート"));
    assert_eq!(
        fs::read(note_path(&root)).unwrap(),
        "hello ノート".as_bytes()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn disabled_save_is_skipped_until_an_edit_enables_it() {
    let root = test_root("focus_order");
    seed_note(&root, b"keep");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    // Pristine form: Save is disabled, so Down from the field skips straight
    // to Reload.
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(RELOAD));

    // Edit one character: the form marks the note unsaved and enables Save.
    let up = intent(koto_core::runtime::text_intent::UP);
    assert_eq!(session.step_frame(up).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('X')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Editing note"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keepX"));

    // Down now lands on the enabled Save.
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(SAVE));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn edit_save_and_reload_round_trip_exact_bytes() {
    let root = test_root("edit_save_reload");
    seed_note(&root, b"hello");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    // Append a multibyte character and save through the button.
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('あ')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_value(FIELD), Some("helloあ"));
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(SAVE));
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note saved"));
    // Save wrote exactly the current valid UTF-8 bytes, disabled itself, and
    // handed focus back to the note field in the same atomic update.
    assert_eq!(fs::read(note_path(&root)).unwrap(), "helloあ".as_bytes());
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));

    // Edit again, then Reload: the field returns to the file content through
    // the ABI and Save disables again.
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('Z')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_value(FIELD), Some("helloあZ"));
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    let right = intent(koto_core::runtime::text_intent::RIGHT);
    assert_eq!(session.step_frame(right).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(RELOAD));
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note loaded"));
    assert_eq!(session.retained_ui_value(FIELD), Some("helloあ"));
    // Save is disabled again: Down from the field skips it.
    let up = intent(koto_core::runtime::text_intent::UP);
    assert_eq!(session.step_frame(up).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(RELOAD));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn submission_saves_from_inside_the_field() {
    let root = test_root("submit_save");
    seed_note(&root, b"s");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('1')).unwrap(),
        VmRunResult::Yielded
    );
    // Enter submits the field value; the app saves it without moving focus.
    let submit = intent(koto_core::runtime::text_intent::NEWLINE);
    assert_eq!(session.step_frame(submit).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_text(STATUS), Some("Note saved"));
    assert_eq!(fs::read(note_path(&root)).unwrap(), b"s1");
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));

    // The note is clean again, so Down skips the re-disabled Save button.
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(RELOAD));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn capacity_rejection_reports_and_keeps_memory_stable() {
    let root = test_root("capacity");
    seed_note(&root, "a".repeat(60).as_bytes());
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    for _ in 0..4 {
        assert_eq!(
            session.step_frame(typed('x')).unwrap(),
            VmRunResult::Yielded
        );
    }
    assert_eq!(session.retained_ui_value(FIELD).unwrap().len(), 64);
    let heap_at_limit = session.budget().heap_bytes_peak;

    assert_eq!(
        session.step_frame(typed('x')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_value(FIELD).unwrap().len(), 64);
    assert_eq!(session.retained_ui_text(STATUS), Some("Capacity full"));
    assert_eq!(session.budget().heap_bytes_peak, heap_at_limit);

    // Recovery: deleting one byte accepts input again without heap growth.
    let backspace = intent(koto_core::runtime::text_intent::BACKSPACE);
    assert_eq!(session.step_frame(backspace).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_value(FIELD).unwrap().len(), 63);
    assert_eq!(
        session.step_frame(typed('y')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_value(FIELD).unwrap().len(), 64);
    assert_eq!(session.budget().heap_bytes_peak, heap_at_limit);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_failure_reports_file_error_without_losing_text() {
    let root = test_root("save_failure");
    seed_note(&root, b"keep");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    // Replace the note with a directory so the sandbox write deterministically
    // fails while the app is running.
    fs::remove_file(note_path(&root)).unwrap();
    fs::create_dir_all(note_path(&root)).unwrap();

    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('X')).unwrap(),
        VmRunResult::Yielded
    );
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );

    // The failure is reported, the edited text survives, and Save remains
    // enabled because the note is still unsaved.
    assert_eq!(session.retained_ui_text(STATUS), Some("File error"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keepX"));
    assert_eq!(session.retained_ui_focused_id(), Some(SAVE));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reload_reports_missing_oversized_invalid_and_normal_deterministically() {
    let root = test_root("reload_matrix");
    seed_note(&root, b"keep");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(session.retained_ui_focused_id(), Some(RELOAD));

    // Missing: the file vanished; the field keeps the unsaved text.
    fs::remove_file(note_path(&root)).unwrap();
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("No note file"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keep"));

    // Oversized: more bytes than the bounded field accepts.
    seed_note(&root, "b".repeat(65).as_bytes());
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note too large"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keep"));

    // Invalid: not UTF-8.
    seed_note(&root, &[0xff, 0xfe, 0x41]);
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note not UTF-8"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keep"));

    // Normal: the field updates through the ABI.
    seed_note(&root, b"fresh");
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note loaded"));
    assert_eq!(session.retained_ui_value(FIELD), Some("fresh"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exit_intent_and_cancel_both_leave_cleanly() {
    let root = test_root("exit_paths");
    seed_note(&root, b"keep");

    // F10 / Shift+F5: the lifecycle intent exits on the same frame.
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    assert_eq!(
        session
            .step_frame(intent(koto_core::runtime::text_intent::EXIT))
            .unwrap(),
        VmRunResult::Exited(0)
    );

    // Cancel: the first press while editing only leaves the field; the second
    // reaches the app as a semantic Cancelled event and exits.
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    let cancel = intent(koto_core::runtime::text_intent::CANCEL);
    assert_eq!(session.step_frame(cancel).unwrap(), VmRunResult::Yielded);
    assert!(!session.has_exited());
    assert_eq!(session.step_frame(cancel).unwrap(), VmRunResult::Exited(0));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saved_note_persists_across_relaunch_with_a_fresh_session() {
    let root = test_root("relaunch");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    assert_eq!(session.retained_ui_text(STATUS), Some("Note created"));
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        session.step_frame(typed('7')).unwrap(),
        VmRunResult::Yielded
    );
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(STATUS), Some("Note saved"));
    assert_eq!(
        session
            .step_frame(intent(koto_core::runtime::text_intent::EXIT))
            .unwrap(),
        VmRunResult::Exited(0)
    );
    drop(session);

    // A relaunch builds a fresh retained session (initial focus, no stale
    // status) and reads back the persisted bytes.
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    assert_eq!(session.retained_ui_text(STATUS), Some("Note loaded"));
    assert_eq!(
        session.retained_ui_value(FIELD),
        Some(format!("{DEFAULT_NOTE}7").as_str())
    );
    assert_eq!(session.retained_ui_focused_id(), Some(FIELD));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn locale_cycle_relabels_the_form_without_touching_note_bytes() {
    let root = test_root("locale_cycle");
    seed_note(&root, b"keep");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    // The first frame after mount establishes the host locale generation.
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.retained_ui_text(PANEL), Some("File Note"));

    let mut config = ConfigService::new();
    assert!(config.set_locale(Locale::JaJp));
    session.set_config_snapshot(config.snapshot());
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    assert_eq!(session.retained_ui_text(PANEL), Some("ファイルメモ"));
    assert_eq!(session.retained_ui_text(FIELD), Some("メモ"));
    assert_eq!(session.retained_ui_text(SAVE), Some("保存"));
    assert_eq!(session.retained_ui_text(RELOAD), Some("再読込"));
    assert_eq!(session.retained_ui_text(STATUS), Some("読込完了"));
    assert_eq!(session.retained_ui_value(FIELD), Some("keep"));

    // The deterministic ASCII pseudolocale expands every label by 35-50%
    // while the saved note bytes stay untouched.
    assert!(config.set_locale(Locale::QpsPloc));
    session.set_config_snapshot(config.snapshot());
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    for (widget, english, pseudo) in [
        (PANEL, "File Note", "[File Note!!]"),
        (FIELD, "Note", "[Note]"),
        (SAVE, "Save", "[Save]"),
        (RELOAD, "Reload", "[Reload!]"),
        (STATUS, "Note loaded", "[Note loaded!!]"),
    ] {
        assert_eq!(session.retained_ui_text(widget), Some(pseudo));
        let expansion = pseudo.len() * 100 / english.len();
        assert!(
            (135..=150).contains(&expansion),
            "{widget}: {pseudo:?} expands {english:?} by {expansion}%"
        );
    }
    assert_eq!(session.retained_ui_value(FIELD), Some("keep"));
    assert_eq!(fs::read(note_path(&root)).unwrap(), b"keep");

    // Unknown tags fall back to English.
    assert!(config.set_locale(Locale::EnUs));
    session.set_config_snapshot(config.snapshot());
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    assert_eq!(session.retained_ui_text(PANEL), Some("File Note"));
    assert_eq!(session.retained_ui_text(STATUS), Some("Note loaded"));
    assert_eq!(fs::read(note_path(&root)).unwrap(), b"keep");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn translations_are_text_assets_not_bytecode_rodata() {
    const BYTECODE: &[u8] = include_bytes!("../../../package_inputs/bytecode/sample_file_note.kbc");
    for embedded in [
        "File Note".as_bytes(),
        "ファイルメモ".as_bytes(),
        "[File Note!!]".as_bytes(),
        "Note loaded".as_bytes(),
    ] {
        assert!(!BYTECODE
            .windows(embedded.len())
            .any(|window| window == embedded));
    }
    for locale in [
        include_str!("../../../apps/samples/file_note/locales/en-US.txt"),
        include_str!("../../../apps/samples/file_note/locales/ja-JP.txt"),
        include_str!("../../../apps/samples/file_note/locales/qps-ploc.txt"),
    ] {
        let lines: Vec<_> = locale.lines().collect();
        assert_eq!(lines.len(), 13);
        assert!(lines.iter().all(|line| !line.is_empty()));
    }
}

#[test]
fn trace_pins_mount_damage_idle_zero_work_and_bounded_edit_damage() {
    let root = test_root("trace");
    seed_note(&root, b"keep");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);

    // The mount frame presents the full panel once.
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(session.ui_presented_damage(), &[(0, 0, 320, 320)]);

    // An unchanged form yields with zero repaint work.
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.ui_present_calls_this_frame(), 0);
    assert_eq!(session.ui_paint_count_this_frame(), 0);
    assert!(session.ui_presented_damage().is_empty());
    assert!(session.ui_polled_events().is_empty());

    // Entering the field raises no semantic event; its retained damage rides
    // the next present instead of forcing an idle repaint (same host policy
    // the Gallery pins for composition-only TextField changes).
    assert_eq!(
        session.step_frame(activate()).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.ui_present_calls_this_frame(), 0);
    assert_eq!(session.ui_paint_count_this_frame(), 0);
    assert!(session.ui_presented_damage().is_empty());
    assert!(session.ui_polled_events().is_empty());

    // The first keystroke changes field, status, and Save enabled state; the
    // presented damage covers exactly those three component bounds.
    assert_eq!(
        session.step_frame(typed('X')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.ui_polled_events(), &[(3, FIELD, 5, 5)]);
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(
        session.ui_presented_damage(),
        &[
            (12, 56, 296, 32),  // TextField
            (12, 28, 296, 20),  // status Label
            (12, 100, 142, 28), // Save button
        ]
    );

    // A second identical status ("Editing note") repaints only the field.
    assert_eq!(
        session.step_frame(typed('Y')).unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(session.ui_presented_damage(), &[(12, 56, 296, 32)]);

    // Focus movement damages only the field it left and the button it enters.
    let down = intent(koto_core::runtime::text_intent::DOWN);
    assert_eq!(session.step_frame(down).unwrap(), VmRunResult::Yielded);
    assert_eq!(
        session.ui_polled_events(),
        &[(8, SAVE, i32::from(FIELD), 0)]
    );
    assert_eq!(session.ui_present_calls_this_frame(), 1);
    assert_eq!(session.ui_paint_count_this_frame(), 1);
    assert_eq!(
        session.ui_presented_damage(),
        &[(12, 56, 296, 32), (12, 100, 142, 28)]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn frames_match_320_square_locale_goldens() {
    let root = test_root("goldens");
    seed_note(&root, b"golden note");
    let mut session = BytecodeAppSession::launch(&root, APP_ID).unwrap();
    boot(&mut session);
    assert_eq!(
        session.step_frame(VmInputSnapshot::empty()).unwrap(),
        VmRunResult::Yielded
    );
    let english = frame_hash(&session);

    let mut config = ConfigService::new();
    assert!(config.set_locale(Locale::JaJp));
    session.set_config_snapshot(config.snapshot());
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    let japanese = frame_hash(&session);

    assert!(config.set_locale(Locale::QpsPloc));
    session.set_config_snapshot(config.snapshot());
    for _ in 0..2 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
    }
    let pseudo = frame_hash(&session);

    println!("file note en={english:#018x} ja={japanese:#018x} qps={pseudo:#018x}");
    assert_eq!(english, ENGLISH_GOLDEN);
    assert_eq!(japanese, JAPANESE_GOLDEN);
    assert_eq!(pseudo, PSEUDO_GOLDEN);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scripted_scenario_stays_in_budget_without_low_level_draws() {
    let root = test_root("scenario");
    let inputs = parse_app_script(INTERACTION).expect("valid File Note scenario");
    let report = run_app_scenario(&root, APP_ID, &inputs).expect("File Note run");

    assert_eq!(report.frames, inputs.len());
    assert_eq!(report.result, VmRunResult::Yielded);
    assert_eq!(report.budget.heap_budget, Some(16_384));
    assert!(report.budget.heap_bytes_peak <= report.budget.heap_request);
    assert!(report.budget.frame_fuel_peak <= report.budget.frame_fuel_cap);
    assert!(report.budget.host_calls_per_frame_peak <= 10);
    assert_eq!(
        report.budget.ui_session_sram_bytes,
        koto_core::UI_SESSION_SRAM_BYTES
    );
    assert!(report.budget.ui_session_sram_bytes <= 8_192);
    println!(
        "file note scenario heap={} fuel={} host_calls={} commands={}",
        report.budget.heap_bytes_peak,
        report.budget.frame_fuel_peak,
        report.budget.host_calls_per_frame_peak,
        report.budget.ui_render_commands_peak
    );
    // KotoUI owns rendering; the bytecode never emits low-level draw calls.
    assert_eq!(report.budget.draw_rects_peak, 0);
    assert_eq!(report.budget.draw_pixels_peak, 0);
    assert_eq!(report.budget.text_draws_peak, 0);
    // The scenario edited then reloaded, so the note bytes end unchanged.
    assert_eq!(fs::read(note_path(&root)).unwrap(), DEFAULT_NOTE.as_bytes());

    fs::remove_dir_all(root).unwrap();
}
