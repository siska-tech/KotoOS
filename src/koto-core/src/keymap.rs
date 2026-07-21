//! PicoCalc keyboard-bridge scan-code → app-input mapping (KOTO-0177).
//!
//! The firmware turns keyboard-bridge key events into the same
//! `text_intent` / typed-codepoint stream that KotoSim produces from PC keys
//! (`src/koto-sim/src/window.rs`, `poll_app_input`). The two targets must
//! agree on the semantics — in particular **F10 is the only key that delivers
//! `text_intent::EXIT`** — or apps behave differently per target. `koto-pico`
//! is excluded from the workspace default members and only builds for the
//! device target, so the pure mapping lives here where the parity tests run
//! on the host.

use crate::runtime::text_intent;

/// Intent bits delivered to apps for a pressed keyboard-bridge key event.
///
/// Scan codes follow the validated PicoCalc keyboard-bridge assignment
/// (KOTO-0025; named constants live in `koto-pico`'s `keyboard` module):
/// `0xb4..=0xb7` arrows, `0x81`-based function keys, `0xa2/0xa3` shifts,
/// `0xa5` Ctrl, `0xb1` Esc.
///
/// Esc and Ctrl+G map to CANCEL. The simulator uses Ctrl+G because its Esc closes
/// the window; PicoCalc keyboard firmware reports Ctrl+G as ASCII BEL (`0x07`).
/// Esc additionally acts as game
/// button B through the firmware's held-key game-pad bits; intents and pad
/// bits coexist, exactly like Enter (NEWLINE intent + button A).
///
/// EXIT (the sim's F10) is carried by **`0x90`** — the PicoCalc keyboard has
/// no dedicated F10 key; F10 is the shifted legend on the F5 keycap, and the
/// bridge shift-translates the whole shifted plane itself before the codes
/// reach us. `0x90` is the device-measured code for Shift+F5 (`phase=180 key`
/// capture, 2026-07-11); it also confirms the bridge's decimal-looking hex
/// numbering (F9 = `0x89` jumps to F10 = `0x90`). The `0x81`-block
/// extrapolation `0x8a` never arrives but stays mapped in case an unshifted
/// F10 exists on another keyboard revision (KOTO-0177).
///
/// The other shifted legends arrive the same way as dedicated codes:
/// Shift+Tab = Home `0xd2`, Shift+Del = End `0xd5` (plain Del is `0xd4`) —
/// ClockworkPi keyboard-firmware protocol values, device-unverified; a
/// `phase=180 key` capture settles any doubt. Shift+Esc = Break `0xd0` and
/// F6–F9 (`0x86..=0x89`) intentionally deliver nothing: the sim assigns no
/// intent to them either, so "no intent" *is* the parity.
pub const fn intent_for_key(key: u8) -> u32 {
    match key {
        0xb4 => text_intent::LEFT,
        0xb7 => text_intent::RIGHT,
        0xb5 => text_intent::UP,
        0xb6 => text_intent::DOWN,
        0x0a => text_intent::NEWLINE,
        0x08 => text_intent::BACKSPACE,
        0x09 | b' ' => text_intent::CONVERT,
        0x81 => text_intent::IME_TOGGLE, // F1
        0xa2 => text_intent::SHIFT,
        0x07 | 0xb1 => text_intent::CANCEL, // Ctrl+G, Esc
        0xa3 => text_intent::SHIFT,
        0x82 => text_intent::SAVE, // F2
        0x84 => text_intent::OPEN, // F4
        0x85 => text_intent::NEW,  // F5
        // F10 (Shift+F5 keycap legend): 0x90 measured on device; 0x8a is the
        // unobserved protocol-block assumption, kept as a harmless alias.
        0x90 | 0x8a => text_intent::EXIT,
        0xd4 => text_intent::DELETE, // Del
        0xd2 => text_intent::HOME,   // Home (Shift+Tab keycap legend)
        0xd5 => text_intent::END,    // End (Shift+Del keycap legend)
        _ => 0,
    }
}

/// Whether a PicoCalc key closes the native KotoConfig surface.
///
/// F1 toggles the surface it opened; F10 reuses the same device-verified EXIT
/// carriers as apps. Other application intents do not leak into KotoConfig.
pub const fn is_config_exit_key(key: u8) -> bool {
    key == 0x81 || intent_for_key(key) & text_intent::EXIT != 0
}

/// Typed codepoint delivered to apps for a pressed keyboard-bridge key event,
/// or `0` when the key types nothing.
///
/// `is_ascii_graphic()` is `0x21..=0x7e`, which excludes Space (`0x20`) — so
/// Space is admitted explicitly. Without it the host never delivers codepoint
/// 32, and apps that read it (KotoBlocks' Space hard drop, or typing a space
/// into the editor) get nothing. Matches the simulator, where `Key::Space`
/// maps to `' '` (koto-sim window `typed_char`).
pub const fn typed_codepoint_for_key(key: u8) -> u32 {
    if key.is_ascii_graphic() || key == b' ' {
        key as u32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KotoSim maps only `Key::F10` to EXIT (`poll_app_input`); the firmware
    /// must match. This is the KOTO-0177 parity contract — the bring-up shim
    /// that also exited on X/Esc must never come back. EXIT's carriers are
    /// exactly the F10 codes: 0x90 (device-measured Shift+F5, the physical
    /// F10 legend) and the 0x8a protocol alias.
    #[test]
    fn exit_is_delivered_only_by_f10() {
        for key in 0..=u8::MAX {
            let exits = intent_for_key(key) & text_intent::EXIT != 0;
            assert_eq!(exits, key == 0x90 || key == 0x8a, "key {key:#04x}");
        }
    }

    #[test]
    fn config_exits_on_f1_and_f10_only() {
        for key in 0..=u8::MAX {
            assert_eq!(
                is_config_exit_key(key),
                matches!(key, 0x81 | 0x8a | 0x90),
                "key {key:#04x}"
            );
        }
    }

    #[test]
    fn escape_delivers_cancel_not_exit() {
        assert_eq!(intent_for_key(0xb1), text_intent::CANCEL);
        assert_eq!(intent_for_key(0x07), text_intent::CANCEL);
        assert_eq!(intent_for_key(0xa5), 0);
    }

    #[test]
    fn skk_keys_use_space_enter_and_ctrl_g_semantics() {
        assert_eq!(intent_for_key(b' '), text_intent::CONVERT);
        assert_eq!(intent_for_key(0x0a), text_intent::NEWLINE);
        assert_eq!(intent_for_key(0xa3), text_intent::SHIFT);
    }

    /// The bridge translates the shifted plane itself: Shift+Tab arrives as
    /// Home, Shift+Del as End; plain Del is its own code. The sim delivers
    /// the same three intents from PC Home/End/Delete keys (`poll_app_input`),
    /// so dropping them was a parity gap. Shift+Esc (Break) and F6–F9 carry
    /// no intent on either target.
    #[test]
    fn shifted_legends_match_sim_intents() {
        assert_eq!(intent_for_key(0xd2), text_intent::HOME);
        assert_eq!(intent_for_key(0xd5), text_intent::END);
        assert_eq!(intent_for_key(0xd4), text_intent::DELETE);
        assert_eq!(intent_for_key(0xd0), 0); // Break
        for f_key in 0x86..=0x89u8 {
            assert_eq!(intent_for_key(f_key), 0, "F6-F9 {f_key:#04x}");
        }
    }

    /// The shim also suppressed the typed `x` so it could own the key; apps
    /// must receive `x` like any other letter (memo text entry, game input).
    #[test]
    fn x_is_a_plain_typed_character() {
        assert_eq!(intent_for_key(b'x'), 0);
        assert_eq!(typed_codepoint_for_key(b'x'), u32::from(b'x'));
    }

    #[test]
    fn typed_codepoints_admit_graphic_ascii_and_space_only() {
        assert_eq!(typed_codepoint_for_key(b' '), 32);
        assert_eq!(typed_codepoint_for_key(b'a'), u32::from(b'a'));
        // Control codes and scan codes above ASCII type nothing.
        assert_eq!(typed_codepoint_for_key(0x08), 0);
        assert_eq!(typed_codepoint_for_key(0x0a), 0);
        assert_eq!(typed_codepoint_for_key(0x8a), 0);
        assert_eq!(typed_codepoint_for_key(0xb1), 0);
    }
}
