//! PicoCalc product-firmware support, split by responsibility (KOTO-0129).
//!
//! The `koto_firmware` binary stays a thin entry point that owns the `StaticCell`
//! buffers and the embassy `main`; everything else — boot constants, UART
//! diagnostics, SD storage, launcher preferences, shell/app rendering, the app
//! `VmHost`, the bytecode run loop, and power polling — lives in these modules
//! and operates on mutable references the binary hands in. This module also keeps
//! the input normalization and KPA-manifest parsing shared by the main loop.

use koto_core::{
    parse_manifest_fetch_permission, Buttons, InputState, ManifestFetchPermission, ManifestFields,
    PackageIconStyle, PackageIconTheme, PackageInfo, PackageManifest,
};

use crate::keyboard::{
    HeldKeys, KeyEvent, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_STATE_PRESSED, KEY_UP,
};

pub mod app_host;
pub mod app_render;
pub mod app_runtime;
pub mod arena_future;
pub mod audio;
pub mod audio_cues;
pub mod audio_residency;
pub mod audio_scratch;
pub mod config;
pub mod config_render;
pub mod config_store;
pub mod diag;
pub mod display_service;
#[cfg(any(
    feature = "app_fetch_tls_socket_adapter_probe",
    feature = "app_fetch_https"
))]
pub mod fetch_tls_adapter;
#[cfg(any(feature = "app_fetch_tls_verifier_probe", feature = "app_fetch_https"))]
pub mod fetch_tls_verifier;
// KOTO-0245 product HTTPS session: the SPKI-pinned TLS 1.3 + streaming-HTTP
// pump that occupies the TLS/audio-exclusive workspace ownership interval.
#[cfg(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
pub mod fetch_https;
#[cfg(all(feature = "app_fetch_https", feature = "mcu-rp235xa"))]
pub mod fetch_tls_workspace;
// KOTO-0239 NetworkService binding. Only meaningful on radio boards with the
// IP stack linked in.
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
pub mod network;
pub mod power;
pub mod resident;
// KOTO-0240 Wi-Fi credential persistence. Only built when the network service
// is linked; offline product builds link none of it.
#[cfg(feature = "network_service")]
pub mod secret_store;
pub mod shell_prefs;
pub mod shell_render;
pub mod spi_bench;
pub mod splash_render;
pub mod stack_canary;
// KOTO-0245 dedicated-stack trampoline for the TLS handshake crypto.
#[cfg(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
pub mod stack_switch;
pub mod storage;
// KOTO-0227 five-minute WifiStreamAudio product-path soak (validation only).
#[cfg(all(feature = "wifi_stream_soak_probe", feature = "board-picocalc-picow"))]
pub mod stream_soak;
pub mod wifi_residency;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FirmwareInput {
    previous: Buttons,
}

impl FirmwareInput {
    pub const fn new() -> Self {
        Self {
            previous: Buttons {
                up: false,
                down: false,
                left: false,
                right: false,
                confirm: false,
                cancel: false,
                menu: false,
                action_a: false,
                action_b: false,
                action_x: false,
                action_y: false,
                shift: false,
            },
        }
    }

    pub fn sample(&mut self, held: &HeldKeys, latest: Option<KeyEvent>) -> InputState {
        let current = buttons_from_held(held);
        let input = InputState {
            held: current,
            pressed: button_delta(current, self.previous),
            released: button_delta(self.previous, current),
            raw_keycode: latest.map(|event| u32::from(event.key)),
            unicode_codepoint: latest
                .filter(|event| event.state == KEY_STATE_PRESSED)
                .and_then(|event| char::from_u32(u32::from(event.key)))
                .filter(|character| !character.is_control()),
        };
        self.previous = current;
        input
    }
}

fn buttons_from_held(held: &HeldKeys) -> Buttons {
    let has = |key| held.as_slice().contains(&key);
    Buttons {
        up: has(KEY_UP),
        down: has(KEY_DOWN),
        left: has(KEY_LEFT),
        right: has(KEY_RIGHT),
        confirm: has(b'z') || has(0x0a),
        cancel: has(b'x') || has(0xb1),
        menu: has(0xb1),
        action_a: has(b'z'),
        action_b: has(b'x'),
        action_x: has(b'a'),
        action_y: has(b's'),
        shift: has(0xa2) || has(0xa3),
    }
}

fn button_delta(current: Buttons, previous: Buttons) -> Buttons {
    Buttons {
        up: current.up && !previous.up,
        down: current.down && !previous.down,
        left: current.left && !previous.left,
        right: current.right && !previous.right,
        confirm: current.confirm && !previous.confirm,
        cancel: current.cancel && !previous.cancel,
        menu: current.menu && !previous.menu,
        action_a: current.action_a && !previous.action_a,
        action_b: current.action_b && !previous.action_b,
        action_x: current.action_x && !previous.action_x,
        action_y: current.action_y && !previous.action_y,
        shift: current.shift && !previous.shift,
    }
}

/// Extract the launcher fields from a bounded KPA manifest read.
///
/// Full launch validation remains in the portable manifest model. The device
/// shell needs only `app_id` and `name` while scanning the SD card, so this
/// parser deliberately accepts only unescaped JSON strings for those fields.
pub fn parse_package_summary(bytes: &[u8]) -> Option<PackageInfo> {
    let text = core::str::from_utf8(bytes).ok()?;
    let version = json_number(text, "version")?;
    // Unlike the launcher's deliberately small flat field reader, this parser
    // structurally walks the complete JSON value. It rejects malformed,
    // duplicate, oversized, wildcard, or version-mismatched Fetch permissions
    // before a package can enter the device catalog.
    let fetch = parse_manifest_fetch_permission(text, version).ok()?;
    PackageManifest::new(ManifestFields {
        format: json_string(text, "format")?,
        version,
        app_id: json_string(text, "app_id")?,
        name: json_string(text, "name")?,
        runtime: json_string(text, "runtime")?,
        entry: json_string(text, "entry")?,
        icon: json_optional_string(text, "icon"),
        shell_icon: parse_shell_icon_theme(text),
        fs_permission: json_optional_string(text, "fs"),
        network_permission: fetch.legacy,
        sram_work_bytes: None,
        psram_cache_bytes: None,
        description: json_optional_string(text, "description"),
        category: json_optional_string(text, "category"),
    })
    .ok()
    .map(PackageManifest::package)
}

/// Rebuild the full fixed-capacity Fetch permission at app launch. The shell
/// catalog deliberately retains only [`PackageInfo`]; keeping every package's
/// origin and pin tables there would multiply the SRAM cost by `MAX_PACKAGES`.
///
/// Writes through `out` (KOTO-0252): returning the ~1.3 KiB permission by
/// value stacked another copy on the launch path's frames. On `false`, `out`
/// holds an unspecified partial value and must not be used.
pub(crate) fn parse_package_fetch_permission_into(
    bytes: &[u8],
    out: &mut ManifestFetchPermission,
) -> bool {
    let Ok(text) = core::str::from_utf8(bytes) else {
        return false;
    };
    let Some(version) = json_number(text, "version") else {
        return false;
    };
    koto_core::parse_manifest_fetch_permission_into(text, version, out).is_ok()
}

fn json_optional_string<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    json_string(text, key)
}

/// Parse the manifest `shell_icon` theme (KOTO-0122). The theme keys are unique
/// within a manifest, so each is matched with the flat string scanner instead of
/// a nested-object parser. Mirrors KotoSim's `parse_shell_icon`: an absent or
/// malformed theme yields `None`, which drives the deterministic fallback icon.
fn parse_shell_icon_theme(text: &str) -> Option<PackageIconTheme> {
    let style = match json_string(text, "style")? {
        "mask" => PackageIconStyle::Mask,
        _ => return None,
    };
    Some(PackageIconTheme {
        style,
        background: parse_hex_rgb565(json_string(text, "background")?)?,
        primary: parse_hex_rgb565(json_string(text, "primary")?)?,
        secondary: parse_hex_rgb565(json_string(text, "secondary")?)?,
        accent: parse_hex_rgb565(json_string(text, "accent")?)?,
        highlight: parse_hex_rgb565(json_string(text, "highlight")?)?,
        shadow: parse_hex_rgb565(json_string(text, "shadow")?)?,
    })
}

/// Convert a `#RRGGBB` string to RGB565, matching KotoSim's `required_rgb565`.
fn parse_hex_rgb565(text: &str) -> Option<u16> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    let r = (rgb >> 16) & 0xff;
    let g = (rgb >> 8) & 0xff;
    let b = rgb & 0xff;
    Some((((r * 31 / 255) << 11) | ((g * 63 / 255) << 5) | (b * 31 / 255)) as u16)
}

fn json_string<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let value = json_value_start(text, key)?;
    let rest = text.get(value..)?;
    if !rest.starts_with('"') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => return rest.get(1..index),
            b'\\' => return None,
            _ => index += 1,
        }
    }
    None
}

fn json_number(text: &str, key: &str) -> Option<u32> {
    let start = json_value_start(text, key)?;
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == start {
        return None;
    }
    text.get(start..end)?.parse().ok()
}

fn json_value_start(text: &str, key: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut search = 0usize;
    while search < bytes.len() {
        let quote = bytes[search..].iter().position(|byte| *byte == b'"')? + search;
        let key_start = quote + 1;
        let key_end = bytes[key_start..].iter().position(|byte| *byte == b'"')? + key_start;
        if text.get(key_start..key_end)? == key {
            let mut cursor = key_end + 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b':') {
                return None;
            }
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            return Some(cursor);
        }
        search = key_end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::{KEY_STATE_PRESSED, KEY_STATE_RELEASED};

    #[test]
    fn reports_button_edges_and_text_once() {
        let mut held = HeldKeys::new();
        let mut input = FirmwareInput::new();
        let press = KeyEvent {
            state: KEY_STATE_PRESSED,
            key: b'z',
        };
        held.apply(press);

        let first = input.sample(&held, Some(press));
        assert!(first.held.confirm);
        assert!(first.pressed.confirm);
        assert_eq!(first.unicode_codepoint, Some('z'));

        let second = input.sample(&held, None);
        assert!(second.held.confirm);
        assert!(!second.pressed.confirm);

        let release = KeyEvent {
            state: KEY_STATE_RELEASED,
            key: b'z',
        };
        held.apply(release);
        let third = input.sample(&held, Some(release));
        assert!(third.released.confirm);
        assert_eq!(third.unicode_codepoint, None);
    }

    #[test]
    fn maps_validated_game_controls() {
        let mut held = HeldKeys::new();
        held.apply(KeyEvent {
            state: KEY_STATE_PRESSED,
            key: KEY_RIGHT,
        });
        held.apply(KeyEvent {
            state: KEY_STATE_PRESSED,
            key: b'a',
        });

        let state = FirmwareInput::new().sample(&held, None);
        assert!(state.held.right);
        assert!(state.held.action_x);
    }

    #[test]
    fn parses_bounded_package_summary() {
        let package = parse_package_summary(
            br#"{"format":"kpa-manifest","version":1,"app_id":"dev.koto.memo","name":"Koto Memo","runtime":"kotoruntime-bytecode","entry":"bytecode/memo.kbc","description":"Memo","category":"Apps"}"#,
        )
        .unwrap();
        assert_eq!(package.app_id(), "dev.koto.memo");
        assert_eq!(package.name(), "Koto Memo");
    }

    #[test]
    fn device_summary_validates_versioned_fetch_permission() {
        let package = parse_package_summary(
            br#"{"format":"kpa-manifest","version":2,"app_id":"dev.koto.fetch","name":"Fetch","runtime":"kotoruntime-bytecode","entry":"bytecode/fetch.kbc","permissions":{"network":{"origins":["https://weather.example"]}}}"#,
        )
        .unwrap();
        assert_eq!(package.app_id(), "dev.koto.fetch");
        assert_eq!(package.network_permission(), None);

        assert!(parse_package_summary(
            br#"{"format":"kpa-manifest","version":2,"app_id":"dev.koto.fetch","name":"Fetch","runtime":"kotoruntime-bytecode","entry":"bytecode/fetch.kbc","permissions":{"network":{"origins":["https://*.example"]}}}"#,
        )
        .is_none());
        assert!(parse_package_summary(
            br#"{"format":"kpa-manifest","version":1,"app_id":"dev.koto.fetch","name":"Fetch","runtime":"kotoruntime-bytecode","entry":"bytecode/fetch.kbc","permissions":{"network":{"origins":[]}}}"#,
        )
        .is_none());

        assert!(parse_package_summary(
            br#"{"format":"kpa-manifest","version":2,"app_id":"dev.koto.fetch","name":"Fetch","runtime":"kotoruntime-bytecode","entry":"bytecode/fetch.kbc","permissions":{"network":{"origins":[{"origin":"https://secure.example","spki_sha256":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}]}}}"#,
        )
        .is_some());
        assert!(parse_package_summary(
            br#"{"format":"kpa-manifest","version":2,"app_id":"dev.koto.fetch","name":"Fetch","runtime":"kotoruntime-bytecode","entry":"bytecode/fetch.kbc","permissions":{"network":{"origins":[{"origin":"https://secure.example","spki_sha256":[]}]}}}"#,
        )
        .is_none());
    }

    #[test]
    fn rejects_wrong_format_and_escaped_summary_fields() {
        assert!(parse_package_summary(
            br#"{"format":"other","version":1,"app_id":"dev.koto.memo","name":"Memo","runtime":"kotoruntime-bytecode","entry":"bytecode/memo.kbc"}"#
        )
        .is_none());
        assert!(parse_package_summary(
            br#"{"format":"kpa-manifest","version":1,"app_id":"dev.koto.memo","name":"Koto\u0020Memo","runtime":"kotoruntime-bytecode","entry":"bytecode/memo.kbc"}"#
        )
        .is_none());
    }

    #[test]
    fn parses_manifest_shell_icon_theme() {
        let theme = parse_shell_icon_theme(
            r##"{"shell_icon":{"style":"mask","background":"#F8F5E8","primary":"#273752","secondary":"#FFFFFF","accent":"#D64A4A","highlight":"#F6D86B","shadow":"#5A8FCB"}}"##,
        )
        .unwrap();
        assert_eq!(theme.style, PackageIconStyle::Mask);
        assert_eq!(theme.secondary, 0xFFFF); // #FFFFFF
        assert_eq!(parse_hex_rgb565("#000000"), Some(0x0000));
        assert_eq!(parse_hex_rgb565("#FF0000"), Some(0xF800));
    }

    #[test]
    fn rejects_missing_or_unknown_shell_icon_theme() {
        assert!(parse_shell_icon_theme(r#"{"name":"No Theme"}"#).is_none());
        assert!(parse_shell_icon_theme(
            r##"{"shell_icon":{"style":"fancy","background":"#F8F5E8"}}"##
        )
        .is_none());
        assert!(parse_hex_rgb565("F8F5E8").is_none());
        assert!(parse_hex_rgb565("#F8F").is_none());
    }
}
