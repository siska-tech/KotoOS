//! Native, allocation-free KotoConfig `network.wifi` surface (KOTO-0241).

use core::fmt::{self, Write};

use koto_ui::{PaintError, Painter, Panel, Rgb565, TextAlign, TextRun, Theme, UiContext, UiRect};

use crate::{
    Locale, NetworkError, NetworkSnapshot, ScanResult, Security, WifiIntent, WifiKey,
    WifiPageController, WifiPageState, CREDENTIAL_MAX_BYTES, CREDENTIAL_MIN_BYTES,
    SCAN_RESULTS_MAX,
};

pub const KOTOCONFIG_WIFI_SURFACE: UiRect = UiRect::new(0, 0, 320, 320);
const PANEL: UiRect = UiRect::new(8, 8, 304, 304);
const STATE: UiRect = UiRect::new(20, 38, 280, 20);
const STATUS: UiRect = UiRect::new(20, 60, 280, 20);
const RESULTS: UiRect = UiRect::new(20, 84, 280, 192);
const HELP: UiRect = UiRect::new(20, 286, 280, 18);
const ROW_HEIGHT: i32 = 12;

/// Native renderer around the fixed-capacity portable Wi-Fi controller.
pub struct KotoConfigWifiUi {
    controller: WifiPageController,
    context: UiContext<8>,
    locale: Locale,
    snapshot: NetworkSnapshot,
    validation_error: bool,
    initialized: bool,
}

impl KotoConfigWifiUi {
    pub fn new(locale: Locale, snapshot: NetworkSnapshot) -> Self {
        let mut context = UiContext::new(KOTOCONFIG_WIFI_SURFACE, Theme::DARK);
        context.damage(PANEL);
        Self {
            controller: WifiPageController::new(),
            context,
            locale,
            snapshot,
            validation_error: false,
            initialized: false,
        }
    }

    pub fn state(&self) -> WifiPageState {
        self.controller.state()
    }

    pub fn selected(&self) -> u8 {
        self.controller.selected()
    }

    pub fn row_count(&self) -> u8 {
        self.controller.row_count()
    }

    pub fn credential_len(&self) -> u8 {
        self.controller.credential_len()
    }

    pub fn credential(&self) -> &[u8] {
        self.controller.credential()
    }

    pub fn credential_zeroized(&self) -> bool {
        self.controller.credential_zeroized()
    }

    pub fn damaged_rects(&self) -> impl Iterator<Item = UiRect> + '_ {
        self.context.damaged_rects()
    }

    pub fn clear_damage(&mut self) {
        self.context.clear_damage();
    }

    pub fn set_locale(&mut self, locale: Locale) {
        if self.locale != locale {
            self.locale = locale;
            self.context.damage(PANEL);
        }
    }

    /// Reconciles exactly one service snapshot and at most one key per frame.
    pub fn update(
        &mut self,
        snapshot: NetworkSnapshot,
        results: impl Iterator<Item = ScanResult>,
        key: Option<WifiKey>,
    ) -> WifiIntent {
        self.context.clear_damage();
        let before_state = self.controller.state();
        let before_selected = self.controller.selected();
        let before_credential_len = self.controller.credential_len();
        let before_error = self.controller.last_error();
        let before_validation_error = self.validation_error;
        let before_rows = copy_rows(&self.controller);
        let before_snapshot = self.snapshot;

        if matches!(key, Some(WifiKey::Char(_) | WifiKey::Backspace)) {
            self.validation_error = false;
        }
        let invalid_submit = before_state == WifiPageState::CredentialEntry
            && key == Some(WifiKey::Enter)
            && !(CREDENTIAL_MIN_BYTES..=CREDENTIAL_MAX_BYTES)
                .contains(&usize::from(before_credential_len));

        let intent = self.controller.update(&snapshot, results, key);
        self.snapshot = snapshot;
        if invalid_submit {
            self.validation_error = true;
        } else if before_state != self.controller.state() {
            self.validation_error = false;
        }

        let after_rows = copy_rows(&self.controller);
        if !self.initialized || before_state != self.controller.state() {
            self.context.damage(PANEL);
        } else {
            if before_rows != after_rows {
                self.context.damage(RESULTS);
            } else if before_selected != self.controller.selected() {
                self.context.damage(row_rect(before_selected));
                self.context.damage(row_rect(self.controller.selected()));
            }
            if before_credential_len != self.controller.credential_len()
                || before_error != self.controller.last_error()
                || before_validation_error != self.validation_error
                || before_snapshot.retry_count != snapshot.retry_count
                || before_snapshot.deadline_ms_remaining != snapshot.deadline_ms_remaining
            {
                self.context.damage(STATUS);
            }
        }
        self.initialized = true;

        if intent == WifiIntent::Exit {
            self.controller.clear_credential();
        }
        intent
    }

    /// Called immediately after the driver has borrowed credential bytes for a
    /// successful connect submission.
    pub fn submission_complete(&mut self, intent: WifiIntent) {
        if matches!(intent, WifiIntent::Connect { .. } | WifiIntent::Cancel) {
            self.controller.clear_credential();
            self.context.damage(STATUS);
        }
    }

    /// Moves directly from credential entry to connecting for a profile whose
    /// secret remains owned by the platform credential provider.
    pub fn begin_saved_connect(&mut self, result_id: u16, security: Security) -> WifiIntent {
        let intent = self.controller.begin_saved_connect(result_id, security);
        if intent != WifiIntent::None {
            self.validation_error = false;
            self.context.damage(PANEL);
        }
        intent
    }

    pub fn reset(&mut self) {
        self.controller.reset();
        self.validation_error = false;
        self.initialized = false;
        self.context.damage(PANEL);
    }

    pub fn paint(&self, painter: &mut impl Painter, clip: UiRect) -> Result<(), PaintError> {
        let theme = Theme::DARK;
        Panel::new(PANEL)
            .with_title(page_title(self.locale))
            .paint(painter, clip, &theme)?;
        draw(
            painter,
            clip,
            STATE,
            state_text(self.locale, self.state()),
            theme.accent,
        )?;

        let mut status = StackText::<128>::new();
        self.write_status(&mut status);
        draw(
            painter,
            clip,
            STATUS,
            status.as_str(),
            theme.normal.foreground,
        )?;

        if self.state() == WifiPageState::Results {
            for (index, result) in self.controller.rows().enumerate() {
                let row = row_rect(index as u8);
                if index as u8 == self.controller.selected() {
                    painter.fill_rect(clip, row, theme.focused.background)?;
                    painter.draw_focus_mark(clip, row, theme.focus, theme.focus_width)?;
                }
                let mut text = StackText::<128>::new();
                write_ssid(&mut text, result);
                let security = match (self.locale, result.security) {
                    (Locale::JaJp, Security::Open) => " オープン",
                    (_, Security::Open) => " Open",
                    (_, Security::Wpa2PersonalAes) => " WPA2",
                };
                let _ = write!(text, "{security} {}dBm", result.rssi_dbm);
                draw(painter, clip, row, text.as_str(), theme.normal.foreground)?;
            }
        }

        draw(
            painter,
            clip,
            HELP,
            help_text(self.locale, self.state()),
            theme.disabled.foreground,
        )
    }

    fn write_status(&self, out: &mut StackText<128>) {
        if self.validation_error {
            out.push_str(validation_text(self.locale));
            return;
        }
        match self.state() {
            WifiPageState::CredentialEntry => {
                out.push_str(password_text(self.locale));
                out.push_str(": ");
                for _ in 0..self.controller.credential_len() {
                    out.push_char('*');
                }
            }
            WifiPageState::Failed | WifiPageState::RadioUnavailable => {
                out.push_str(error_text(self.locale, self.controller.last_error()));
            }
            WifiPageState::Scanning | WifiPageState::Connecting => {
                let seconds = self.snapshot.deadline_ms_remaining / 1_000;
                let _ = write!(
                    out,
                    "{}  {}:{}  {}:{}s",
                    progress_text(self.locale),
                    retry_label(self.locale),
                    self.snapshot.retry_count,
                    remaining_label(self.locale),
                    seconds
                );
            }
            WifiPageState::Results => {
                let _ = write!(out, "{}: {}", networks_text(self.locale), self.row_count());
            }
            WifiPageState::Connected => out.push_str(connected_text(self.locale)),
            _ => out.push_str(state_text(self.locale, self.state())),
        }
    }
}

fn copy_rows(controller: &WifiPageController) -> [Option<ScanResult>; SCAN_RESULTS_MAX] {
    let mut rows = [None; SCAN_RESULTS_MAX];
    for (slot, result) in rows.iter_mut().zip(controller.rows()) {
        *slot = Some(*result);
    }
    rows
}

fn row_rect(index: u8) -> UiRect {
    UiRect::new(
        RESULTS.x,
        RESULTS.y + i32::from(index) * ROW_HEIGHT,
        RESULTS.w,
        ROW_HEIGHT,
    )
}

fn draw(
    painter: &mut impl Painter,
    clip: UiRect,
    bounds: UiRect,
    text: &str,
    color: Rgb565,
) -> Result<(), PaintError> {
    painter.draw_text(
        clip,
        bounds,
        TextRun {
            text,
            color,
            align: TextAlign::Start,
        },
    )
}

fn write_ssid(out: &mut StackText<128>, result: &ScanResult) {
    if let Ok(text) = core::str::from_utf8(result.ssid.as_bytes()) {
        out.push_str(text);
        return;
    }
    for &byte in result.ssid.as_bytes() {
        if (0x20..=0x7e).contains(&byte) && byte != b'\\' {
            out.push_char(char::from(byte));
        } else {
            let _ = write!(out, "\\x{byte:02X}");
        }
    }
}

struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn push_str(&mut self, text: &str) {
        let available = N.saturating_sub(self.len);
        let mut take = text.len().min(available);
        while take > 0 && !text.is_char_boundary(take) {
            take -= 1;
        }
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
    }

    fn push_char(&mut self, ch: char) {
        let mut encoded = [0; 4];
        self.push_str(ch.encode_utf8(&mut encoded));
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> Write for StackText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_str(text);
        Ok(())
    }
}

fn page_title(locale: Locale) -> &'static str {
    match locale {
        Locale::JaJp => "KotoConfig - Wi-Fi 設定",
        Locale::QpsPloc => "[!! KotoConfig - Wi-Fi settings--- !!]",
        Locale::EnUs => "KotoConfig - Wi-Fi settings",
    }
}

fn state_text(locale: Locale, state: WifiPageState) -> &'static str {
    match (locale, state) {
        (Locale::JaJp, WifiPageState::Disabled) => "Wi-Fi はオフです",
        (Locale::JaJp, WifiPageState::Scanning) => "検索中...",
        (Locale::JaJp, WifiPageState::Results) => "ネットワーク",
        (Locale::JaJp, WifiPageState::CredentialEntry) => "パスワードを入力してください",
        (Locale::JaJp, WifiPageState::Connecting) => "接続中...",
        (Locale::JaJp, WifiPageState::Connected) => "状態",
        (Locale::JaJp, WifiPageState::Failed) => "接続に失敗しました",
        (Locale::JaJp, WifiPageState::ForgetConfirm) => "このネットワークを削除しますか?",
        (Locale::JaJp, WifiPageState::RadioUnavailable) => "Wi-Fi を利用できません",
        (_, WifiPageState::Disabled) => "Wi-Fi is off",
        (_, WifiPageState::Scanning) => "Scanning...",
        (_, WifiPageState::Results) => "Networks",
        (_, WifiPageState::CredentialEntry) => "Please enter password",
        (_, WifiPageState::Connecting) => "Connecting...",
        (_, WifiPageState::Connected) => "Status",
        (_, WifiPageState::Failed) => "Connection failed",
        (_, WifiPageState::ForgetConfirm) => "Forget this network?",
        (_, WifiPageState::RadioUnavailable) => "Wi-Fi unavailable",
    }
}

fn help_text(locale: Locale, state: WifiPageState) -> &'static str {
    match (locale, state) {
        (Locale::JaJp, WifiPageState::Disabled) => "Enter: 有効化  Esc: 戻る",
        (Locale::JaJp, WifiPageState::Scanning | WifiPageState::Connecting) => "Esc: キャンセル",
        (Locale::JaJp, WifiPageState::Results) => "↑↓/Tab: 選択  Enter: 接続  R: 再検索  Esc: 戻る",
        (Locale::JaJp, WifiPageState::CredentialEntry) => "入力/Backspace  Enter: 接続  Esc: 戻る",
        (Locale::JaJp, WifiPageState::Connected) => "Enter: 切断  F: 削除  Esc: 戻る",
        (Locale::JaJp, WifiPageState::Failed) => "Enter: 再試行  Esc: 戻る",
        (Locale::JaJp, WifiPageState::ForgetConfirm) => "←→/Tab: 選択  Enter: 決定  Esc: 中止",
        (Locale::JaJp, WifiPageState::RadioUnavailable) => "Esc: 戻る",
        (_, WifiPageState::Disabled) => "Enter: enable  Esc: back",
        (_, WifiPageState::Scanning | WifiPageState::Connecting) => "Esc: cancel",
        (_, WifiPageState::Results) => "Arrows/Tab: select  Enter: connect  R: rescan  Esc: back",
        (_, WifiPageState::CredentialEntry) => "Type/Backspace  Enter: connect  Esc: back",
        (_, WifiPageState::Connected) => "Enter: disconnect  F: forget  Esc: back",
        (_, WifiPageState::Failed) => "Enter: retry  Esc: back",
        (_, WifiPageState::ForgetConfirm) => "Left/Right/Tab: choose  Enter: confirm  Esc: cancel",
        (_, WifiPageState::RadioUnavailable) => "Esc: back",
    }
}

fn error_text(locale: Locale, error: Option<NetworkError>) -> &'static str {
    use NetworkError as E;
    match (locale, error) {
        (Locale::JaJp, Some(E::Busy)) => "処理中です",
        (Locale::JaJp, Some(E::InvalidInput)) => "入力が無効です",
        (Locale::JaJp, Some(E::UnsupportedSecurity)) => "未対応のセキュリティです",
        (Locale::JaJp, Some(E::RadioUnavailable)) => "無線を利用できません",
        (Locale::JaJp, Some(E::FirmwareUnavailable)) => "無線ファームウェアがありません",
        (Locale::JaJp, Some(E::CredentialStoreUnavailable)) => "認証情報ストアを利用できません",
        (Locale::JaJp, Some(E::AuthenticationFailed)) => "認証に失敗しました",
        (Locale::JaJp, Some(E::NetworkNotFound)) => "ネットワークが見つかりません",
        (Locale::JaJp, Some(E::LinkLost)) => "接続が切れました",
        (Locale::JaJp, Some(E::Timeout)) => "タイムアウトしました",
        (Locale::JaJp, Some(E::Cancelled)) => "キャンセルしました",
        (Locale::JaJp, Some(E::StorageCorrupt)) => "認証情報が破損しています",
        (Locale::JaJp, Some(E::Internal) | None) => "内部エラー",
        (_, Some(E::Busy)) => "Service busy",
        (_, Some(E::InvalidInput)) => "Invalid input",
        (_, Some(E::UnsupportedSecurity)) => "Unsupported security",
        (_, Some(E::RadioUnavailable)) => "Radio unavailable",
        (_, Some(E::FirmwareUnavailable)) => "Radio firmware unavailable",
        (_, Some(E::CredentialStoreUnavailable)) => "Credential store unavailable",
        (_, Some(E::AuthenticationFailed)) => "Authentication failed",
        (_, Some(E::NetworkNotFound)) => "Network not found",
        (_, Some(E::LinkLost)) => "Link lost",
        (_, Some(E::Timeout)) => "Timed out",
        (_, Some(E::Cancelled)) => "Cancelled",
        (_, Some(E::StorageCorrupt)) => "Credential store corrupt",
        (_, Some(E::Internal) | None) => "Internal error",
    }
}

fn validation_text(locale: Locale) -> &'static str {
    match locale {
        Locale::JaJp => "パスワードは8～63文字で入力してください",
        _ => "Password must be 8-63 printable characters",
    }
}
fn password_text(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "パスワード"
    } else {
        "Password"
    }
}

fn connected_text(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "接続済み"
    } else {
        "Connected"
    }
}
fn networks_text(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "件数"
    } else {
        "Networks"
    }
}
fn progress_text(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "進行中"
    } else {
        "In progress"
    }
}
fn retry_label(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "再試行"
    } else {
        "retry"
    }
}
fn remaining_label(locale: Locale) -> &'static str {
    if locale == Locale::JaJp {
        "残り"
    } else {
        "remaining"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OperationState, RadioState, Ssid};
    use core::mem::size_of;
    use koto_ui::{GlyphRun, TextMetrics};

    #[derive(Default)]
    struct CommandCounter(usize);

    impl TextMetrics for CommandCounter {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text.len() as i32 * 6)
        }
    }

    impl Painter for CommandCounter {
        fn fill_rect(&mut self, _: UiRect, _: UiRect, _: Rgb565) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
        fn stroke_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
        fn draw_text(&mut self, _: UiRect, _: UiRect, _: TextRun<'_>) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
        fn draw_glyphs(&mut self, _: UiRect, _: UiRect, _: GlyphRun<'_>) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
        fn draw_focus_mark(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
    }

    fn snapshot(state: OperationState) -> NetworkSnapshot {
        NetworkSnapshot {
            generation: 1,
            request_id: 0,
            radio: if state == OperationState::RadioUnavailable {
                RadioState::Unavailable
            } else {
                RadioState::Enabled
            },
            state,
            connected_result_id: None,
            result_count: 0,
            retry_count: 0,
            deadline_ms_remaining: 30_000,
            last_error: None,
            command_depth: 0,
            event_depth: 0,
        }
    }

    #[test]
    fn invalid_ssid_is_escaped_without_heap_text() {
        let result = ScanResult {
            result_id: 1,
            ssid: Ssid::from_bytes(&[0xff, b'K']),
            bssid: [0; 6],
            rssi_dbm: -40,
            security: Security::Open,
        };
        let mut text = StackText::<128>::new();
        write_ssid(&mut text, &result);
        assert_eq!(text.as_str(), "\\xFFK");
    }

    #[test]
    fn idle_update_has_no_damage_and_locale_preserves_selection() {
        let snap = snapshot(OperationState::Results);
        let rows = [ScanResult {
            result_id: 1,
            ssid: Ssid::from_bytes(b"Koto"),
            bssid: [0; 6],
            rssi_dbm: -40,
            security: Security::Open,
        }];
        let mut ui = KotoConfigWifiUi::new(Locale::EnUs, snap);
        ui.update(snap, rows.into_iter(), None);
        ui.clear_damage();
        ui.update(snap, rows.into_iter(), None);
        assert_eq!(ui.damaged_rects().count(), 0);
        ui.set_locale(Locale::JaJp);
        assert_eq!(ui.selected(), 0);
        assert!(ui.damaged_rects().any(|rect| rect == PANEL));
    }

    #[test]
    fn resident_state_and_credential_are_bounded() {
        assert!(size_of::<WifiPageController>() <= 800);
        assert!(size_of::<KotoConfigWifiUi>() <= 1_024);
        assert_eq!(CREDENTIAL_MAX_BYTES, 63);
        assert_eq!(SCAN_RESULTS_MAX, 16);
        assert_eq!(ROW_HEIGHT * SCAN_RESULTS_MAX as i32, RESULTS.h);
    }

    #[test]
    fn empty_and_full_results_are_bounded_with_targeted_selection_damage() {
        let snap = snapshot(OperationState::Results);
        let mut ui = KotoConfigWifiUi::new(Locale::EnUs, snap);
        ui.update(snap, core::iter::empty(), None);
        assert_eq!(ui.row_count(), 0);

        let rows: [ScanResult; SCAN_RESULTS_MAX] = core::array::from_fn(|index| ScanResult {
            result_id: index as u16 + 1,
            ssid: Ssid::from_bytes(b"Network"),
            bssid: [2, 0, 0, 0, 0, index as u8],
            rssi_dbm: -30 - index as i8,
            security: if index % 2 == 0 {
                Security::Open
            } else {
                Security::Wpa2PersonalAes
            },
        });
        ui.update(snap, rows.into_iter(), None);
        assert_eq!(ui.row_count(), SCAN_RESULTS_MAX as u8);
        ui.clear_damage();
        ui.update(snap, rows.into_iter(), Some(WifiKey::Next));
        let damage: Vec<_> = ui.damaged_rects().collect();
        assert_eq!(damage, vec![UiRect::new(20, 84, 280, 24)]);

        let mut commands = CommandCounter::default();
        ui.paint(&mut commands, KOTOCONFIG_WIFI_SURFACE).unwrap();
        assert!(commands.0 <= 25, "render command budget: {}", commands.0);
        assert!(damage
            .iter()
            .all(|rect| rect.w * rect.h <= PANEL.w * PANEL.h));
    }
}
