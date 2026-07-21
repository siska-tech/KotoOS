//! Portable `network.wifi` KotoConfig page controller (KOTO-0241 core).
//!
//! This is a fixed-capacity, `no_std`, heap-free state machine that turns
//! [`crate::net::NetworkSnapshot`]s and keyboard input into bounded
//! [`WifiIntent`]s for a driver to submit to the `NetworkService`. It never
//! calls CYW43, Embassy, sockets, or storage, and it copies at most one snapshot
//! per frame. The masked credential field is a fixed 63-byte buffer that is
//! zeroized on submission, cancellation, page exit, and capability loss.
//!
//! Rendering is the driver's job: the controller exposes redacted getters
//! (state, result rows, selection, credential length, last error) so a UART or
//! LCD front end can present the page without ever reading credential bytes.

use crate::net::{
    NetworkError, NetworkSnapshot, OperationState, RadioState, ScanResult, Security,
    CREDENTIAL_MAX_BYTES, CREDENTIAL_MIN_BYTES, SCAN_RESULTS_MAX,
};

/// The nine `network.wifi` page states from the KOTO-0224 contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WifiPageState {
    Disabled,
    Scanning,
    Results,
    CredentialEntry,
    Connecting,
    Connected,
    Failed,
    ForgetConfirm,
    RadioUnavailable,
}

/// A single decoded key. Printable ASCII arrives as [`WifiKey::Char`]; the
/// controller interprets it per state (password byte, rescan, forget, ...).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WifiKey {
    Up,
    Down,
    Left,
    Right,
    Next,
    Previous,
    Enter,
    Esc,
    Backspace,
    Char(u8),
}

/// A bounded action for the driver to submit to the `NetworkService`. Credential
/// bytes are never carried here; the driver reads [`WifiPageController::credential`]
/// when it sees [`WifiIntent::Connect`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WifiIntent {
    None,
    Exit,
    EnableRadio,
    Scan,
    Connect { result_id: u16, security: Security },
    Disconnect,
    Forget { profile_id: u16 },
    Cancel,
}

/// The fixed-capacity page controller.
pub struct WifiPageController {
    state: WifiPageState,
    rows: [Option<ScanResult>; SCAN_RESULTS_MAX],
    row_count: u8,
    selected: u8,
    target_result_id: u16,
    target_security: Security,
    credential: [u8; CREDENTIAL_MAX_BYTES],
    credential_len: u8,
    forget_choice_is_forget: bool,
    last_error: Option<NetworkError>,
    has_scanned: bool,
}

impl WifiPageController {
    pub fn new() -> Self {
        Self {
            state: WifiPageState::Disabled,
            rows: [None; SCAN_RESULTS_MAX],
            row_count: 0,
            selected: 0,
            target_result_id: 0,
            target_security: Security::Open,
            credential: [0; CREDENTIAL_MAX_BYTES],
            credential_len: 0,
            forget_choice_is_forget: false,
            last_error: None,
            has_scanned: false,
        }
    }

    // ------------------------------------------------------------- observation

    pub fn state(&self) -> WifiPageState {
        self.state
    }

    pub fn rows(&self) -> impl Iterator<Item = &ScanResult> {
        self.rows[..usize::from(self.row_count)]
            .iter()
            .filter_map(Option::as_ref)
    }

    pub fn row_count(&self) -> u8 {
        self.row_count
    }

    pub fn selected(&self) -> u8 {
        self.selected
    }

    pub fn last_error(&self) -> Option<NetworkError> {
        self.last_error
    }

    /// Length of the masked credential field. The bytes themselves are exposed
    /// only through [`Self::credential`], for the driver's connect submission.
    pub fn credential_len(&self) -> u8 {
        self.credential_len
    }

    /// The staged credential bytes. Intended only for building the connect
    /// `CredentialView`; never render these.
    pub fn credential(&self) -> &[u8] {
        &self.credential[..usize::from(self.credential_len)]
    }

    /// The forget-confirm highlighted choice (`true` = forget).
    pub fn forget_choice_is_forget(&self) -> bool {
        self.forget_choice_is_forget
    }

    /// Whether the credential staging is fully zeroed (test/diagnostic).
    pub fn credential_zeroized(&self) -> bool {
        self.credential_len == 0 && self.credential.iter().all(|&b| b == 0)
    }

    // ------------------------------------------------------------- main update

    /// Reconciles from the latest snapshot and processes at most one key,
    /// returning a single bounded intent. Call once per frame.
    pub fn update(
        &mut self,
        snapshot: &NetworkSnapshot,
        results: impl Iterator<Item = ScanResult>,
        key: Option<WifiKey>,
    ) -> WifiIntent {
        let state_before_reconcile = self.state;
        self.ingest_results(results);
        self.reconcile(snapshot);

        if self.state == WifiPageState::RadioUnavailable
            && state_before_reconcile != WifiPageState::RadioUnavailable
            && matches!(
                state_before_reconcile,
                WifiPageState::Scanning
                    | WifiPageState::CredentialEntry
                    | WifiPageState::Connecting
            )
        {
            return WifiIntent::Cancel;
        }

        let mut intent = WifiIntent::None;
        if let Some(key) = key {
            intent = self.handle_key(key);
        }
        if matches!(intent, WifiIntent::None) {
            intent = self.auto_intent(snapshot);
        }
        intent
    }

    fn ingest_results(&mut self, results: impl Iterator<Item = ScanResult>) {
        for slot in self.rows.iter_mut() {
            *slot = None;
        }
        let mut count = 0usize;
        for result in results.take(SCAN_RESULTS_MAX) {
            self.rows[count] = Some(result);
            count += 1;
        }
        self.row_count = count as u8;
        if self.row_count == 0 {
            self.selected = 0;
        } else if usize::from(self.selected) >= count {
            self.selected = (count - 1) as u8;
        }
    }

    /// Maps the service snapshot into the page state, preserving the UI-only
    /// `CredentialEntry`/`ForgetConfirm` states while the user acts.
    fn reconcile(&mut self, snapshot: &NetworkSnapshot) {
        if matches!(snapshot.radio, RadioState::Unavailable)
            || matches!(snapshot.state, OperationState::RadioUnavailable)
        {
            if !matches!(self.state, WifiPageState::RadioUnavailable) {
                self.zeroize_credential();
                self.has_scanned = false;
                self.state = WifiPageState::RadioUnavailable;
            }
            return;
        }

        match self.state {
            // UI-only states persist until the user acts.
            WifiPageState::CredentialEntry | WifiPageState::ForgetConfirm => {}
            _ => {
                self.state = self.map_service_state(snapshot);
            }
        }

        if !matches!(snapshot.radio, RadioState::Enabled) {
            self.has_scanned = false;
        }
    }

    fn map_service_state(&mut self, snapshot: &NetworkSnapshot) -> WifiPageState {
        match snapshot.state {
            OperationState::Disabled
            | OperationState::RadioEnabling
            | OperationState::RadioDisabling => WifiPageState::Disabled,
            OperationState::Scanning => WifiPageState::Scanning,
            OperationState::Results => WifiPageState::Results,
            OperationState::Connecting => WifiPageState::Connecting,
            OperationState::Connected => WifiPageState::Connected,
            OperationState::Disconnecting | OperationState::Forgetting => WifiPageState::Connecting,
            OperationState::Failed => {
                self.last_error = snapshot.last_error;
                // KOTO-0224: authentication failure returns through
                // CredentialEntry with an empty field.
                if matches!(
                    snapshot.last_error,
                    Some(NetworkError::AuthenticationFailed)
                ) && matches!(
                    self.state,
                    WifiPageState::Connecting | WifiPageState::CredentialEntry
                ) {
                    self.zeroize_credential();
                    WifiPageState::CredentialEntry
                } else {
                    WifiPageState::Failed
                }
            }
            OperationState::RadioUnavailable => WifiPageState::RadioUnavailable,
        }
    }

    /// Auto-scan once after the radio comes up so the user lands on Results.
    fn auto_intent(&mut self, snapshot: &NetworkSnapshot) -> WifiIntent {
        if matches!(snapshot.radio, RadioState::Enabled)
            && matches!(snapshot.state, OperationState::Disabled)
            && matches!(self.state, WifiPageState::Disabled)
            && !self.has_scanned
        {
            self.has_scanned = true;
            self.state = WifiPageState::Scanning;
            return WifiIntent::Scan;
        }
        WifiIntent::None
    }

    // ------------------------------------------------------------- key handling

    fn handle_key(&mut self, key: WifiKey) -> WifiIntent {
        let key = match (self.state, key) {
            (WifiPageState::Results, WifiKey::Next) => WifiKey::Down,
            (WifiPageState::Results, WifiKey::Previous) => WifiKey::Up,
            (WifiPageState::ForgetConfirm, WifiKey::Next) => WifiKey::Right,
            (WifiPageState::ForgetConfirm, WifiKey::Previous) => WifiKey::Left,
            (_, key) => key,
        };
        match self.state {
            WifiPageState::Disabled => match key {
                WifiKey::Enter => WifiIntent::EnableRadio,
                WifiKey::Esc => WifiIntent::Exit,
                _ => WifiIntent::None,
            },
            WifiPageState::Scanning => match key {
                WifiKey::Esc => WifiIntent::Cancel,
                _ => WifiIntent::None,
            },
            WifiPageState::Results => self.handle_results_key(key),
            WifiPageState::CredentialEntry => self.handle_credential_key(key),
            WifiPageState::Connecting => match key {
                WifiKey::Esc => WifiIntent::Cancel,
                _ => WifiIntent::None,
            },
            WifiPageState::Connected => match key {
                WifiKey::Char(b'f') | WifiKey::Char(b'F') => {
                    self.forget_choice_is_forget = false;
                    self.state = WifiPageState::ForgetConfirm;
                    WifiIntent::None
                }
                WifiKey::Enter => WifiIntent::Disconnect,
                WifiKey::Esc => WifiIntent::Exit,
                _ => WifiIntent::None,
            },
            WifiPageState::Failed => match key {
                WifiKey::Enter => {
                    // Retry: auth failures already routed back to CredentialEntry
                    // in reconcile; other failures resubmit the same connect.
                    self.state = WifiPageState::Connecting;
                    WifiIntent::Connect {
                        result_id: self.target_result_id,
                        security: self.target_security,
                    }
                }
                WifiKey::Esc => {
                    self.state = WifiPageState::Results;
                    WifiIntent::None
                }
                _ => WifiIntent::None,
            },
            WifiPageState::ForgetConfirm => self.handle_forget_key(key),
            WifiPageState::RadioUnavailable => match key {
                WifiKey::Esc => WifiIntent::Exit,
                _ => WifiIntent::None,
            },
        }
    }

    fn handle_results_key(&mut self, key: WifiKey) -> WifiIntent {
        match key {
            WifiKey::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                WifiIntent::None
            }
            WifiKey::Down => {
                if self.row_count > 0 && self.selected + 1 < self.row_count {
                    self.selected += 1;
                }
                WifiIntent::None
            }
            WifiKey::Char(b'r') | WifiKey::Char(b'R') => WifiIntent::Scan,
            WifiKey::Enter => {
                let Some(row) = self.selected_row() else {
                    return WifiIntent::None;
                };
                self.target_result_id = row.result_id;
                self.target_security = row.security;
                match row.security {
                    Security::Open => {
                        self.zeroize_credential();
                        self.state = WifiPageState::Connecting;
                        WifiIntent::Connect {
                            result_id: row.result_id,
                            security: Security::Open,
                        }
                    }
                    Security::Wpa2PersonalAes => {
                        self.zeroize_credential();
                        self.state = WifiPageState::CredentialEntry;
                        WifiIntent::None
                    }
                }
            }
            WifiKey::Esc => WifiIntent::Exit,
            _ => WifiIntent::None,
        }
    }

    fn handle_credential_key(&mut self, key: WifiKey) -> WifiIntent {
        match key {
            WifiKey::Char(byte) => {
                // Only printable ASCII is a valid WPA2 passphrase byte.
                if (0x20..=0x7e).contains(&byte)
                    && usize::from(self.credential_len) < CREDENTIAL_MAX_BYTES
                {
                    self.credential[usize::from(self.credential_len)] = byte;
                    self.credential_len += 1;
                }
                WifiIntent::None
            }
            WifiKey::Backspace => {
                if self.credential_len > 0 {
                    self.credential_len -= 1;
                    self.credential[usize::from(self.credential_len)] = 0;
                }
                WifiIntent::None
            }
            WifiKey::Enter => {
                if (CREDENTIAL_MIN_BYTES..=CREDENTIAL_MAX_BYTES)
                    .contains(&usize::from(self.credential_len))
                {
                    self.state = WifiPageState::Connecting;
                    WifiIntent::Connect {
                        result_id: self.target_result_id,
                        security: self.target_security,
                    }
                } else {
                    WifiIntent::None
                }
            }
            WifiKey::Esc => {
                self.zeroize_credential();
                self.state = WifiPageState::Results;
                WifiIntent::None
            }
            _ => WifiIntent::None,
        }
    }

    fn handle_forget_key(&mut self, key: WifiKey) -> WifiIntent {
        match key {
            WifiKey::Left => {
                self.forget_choice_is_forget = false;
                WifiIntent::None
            }
            WifiKey::Right => {
                self.forget_choice_is_forget = true;
                WifiIntent::None
            }
            WifiKey::Enter => {
                if self.forget_choice_is_forget {
                    self.state = WifiPageState::Connecting;
                    WifiIntent::Forget {
                        profile_id: self.target_result_id,
                    }
                } else {
                    self.state = WifiPageState::Connected;
                    WifiIntent::None
                }
            }
            WifiKey::Esc => {
                self.state = WifiPageState::Connected;
                WifiIntent::None
            }
            _ => WifiIntent::None,
        }
    }

    // ------------------------------------------------------------- credential

    /// Clears and zeroizes the credential field. Called by the driver on page
    /// exit and after a successful connect submission.
    pub fn clear_credential(&mut self) {
        self.zeroize_credential();
    }

    /// Resets all page-owned state and volatile credential storage.
    pub fn reset(&mut self) {
        self.zeroize_credential();
        for row in self.rows.iter_mut() {
            *row = None;
        }
        self.row_count = 0;
        self.selected = 0;
        self.target_result_id = 0;
        self.target_security = Security::Open;
        self.forget_choice_is_forget = false;
        self.last_error = None;
        self.has_scanned = false;
        self.state = WifiPageState::Disabled;
    }

    /// Skips credential entry when the platform owns a matching saved profile.
    /// Secret bytes remain behind the credential-provider boundary; the page
    /// records only the visible scan identity and moves to `Connecting`.
    pub fn begin_saved_connect(&mut self, result_id: u16, security: Security) -> WifiIntent {
        let matches_row = self
            .rows
            .iter()
            .flatten()
            .any(|row| row.result_id == result_id && row.security == security);
        if !matches_row || self.state != WifiPageState::CredentialEntry {
            return WifiIntent::None;
        }
        self.target_result_id = result_id;
        self.target_security = security;
        self.zeroize_credential();
        self.state = WifiPageState::Connecting;
        WifiIntent::Connect {
            result_id,
            security,
        }
    }

    fn zeroize_credential(&mut self) {
        for byte in self.credential.iter_mut() {
            unsafe {
                core::ptr::write_volatile(byte, 0);
            }
        }
        self.credential_len = 0;
    }

    fn selected_row(&self) -> Option<ScanResult> {
        self.rows
            .get(usize::from(self.selected))
            .and_then(|row| *row)
    }
}

impl Default for WifiPageController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WifiPageController {
    fn drop(&mut self) {
        self.zeroize_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Ssid, BSSID_BYTES};

    fn snap(
        radio: RadioState,
        state: OperationState,
        err: Option<NetworkError>,
    ) -> NetworkSnapshot {
        NetworkSnapshot {
            generation: 1,
            request_id: 0,
            radio,
            state,
            connected_result_id: None,
            result_count: 0,
            retry_count: 0,
            deadline_ms_remaining: 0,
            last_error: err,
            command_depth: 0,
            event_depth: 0,
        }
    }

    fn result(id: u16, security: Security) -> ScanResult {
        ScanResult {
            result_id: id,
            ssid: Ssid::from_bytes(b"KotoLab"),
            bssid: [0x02, 0, 0, 0, 0, id as u8],
            rssi_dbm: -50,
            security,
        }
    }

    fn no_results() -> core::iter::Empty<ScanResult> {
        core::iter::empty()
    }

    #[test]
    fn enable_then_auto_scan() {
        let mut page = WifiPageController::new();
        let intent = page.update(
            &snap(RadioState::Disabled, OperationState::Disabled, None),
            no_results(),
            Some(WifiKey::Enter),
        );
        assert_eq!(intent, WifiIntent::EnableRadio);
        assert_eq!(page.state(), WifiPageState::Disabled);

        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Disabled, None),
            no_results(),
            None,
        );
        assert_eq!(intent, WifiIntent::Scan);
        assert_eq!(page.state(), WifiPageState::Scanning);

        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Scanning, None),
            no_results(),
            None,
        );
        assert_eq!(intent, WifiIntent::None);
    }

    #[test]
    fn results_selection_and_open_connect() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [
                result(1, Security::Wpa2PersonalAes),
                result(2, Security::Open),
            ]
            .into_iter(),
            None,
        );
        assert_eq!(page.state(), WifiPageState::Results);
        assert_eq!(page.row_count(), 2);

        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [
                result(1, Security::Wpa2PersonalAes),
                result(2, Security::Open),
            ]
            .into_iter(),
            Some(WifiKey::Down),
        );
        assert_eq!(page.selected(), 1);

        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [
                result(1, Security::Wpa2PersonalAes),
                result(2, Security::Open),
            ]
            .into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(
            intent,
            WifiIntent::Connect {
                result_id: 2,
                security: Security::Open
            }
        );
        assert_eq!(page.state(), WifiPageState::Connecting);
    }

    #[test]
    fn wpa2_credential_entry_and_connect() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(page.state(), WifiPageState::CredentialEntry);

        for &b in b"passw0rd" {
            page.update(
                &snap(RadioState::Enabled, OperationState::Results, None),
                [result(1, Security::Wpa2PersonalAes)].into_iter(),
                Some(WifiKey::Char(b)),
            );
        }
        assert_eq!(page.credential_len(), 8);
        assert_eq!(page.credential(), b"passw0rd");

        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Backspace),
        );
        assert_eq!(page.credential_len(), 7);
        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(intent, WifiIntent::None);

        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Char(b'x')),
        );
        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(
            intent,
            WifiIntent::Connect {
                result_id: 1,
                security: Security::Wpa2PersonalAes
            }
        );
        assert_eq!(page.state(), WifiPageState::Connecting);
    }

    #[test]
    fn credential_esc_zeroizes_and_returns() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Char(b'a')),
        );
        assert_eq!(page.credential_len(), 1);
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Esc),
        );
        assert_eq!(page.state(), WifiPageState::Results);
        assert!(page.credential_zeroized());
    }

    #[test]
    fn connecting_to_connected_and_disconnect() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Connecting, None),
            no_results(),
            None,
        );
        assert_eq!(page.state(), WifiPageState::Connecting);
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            None,
        );
        assert_eq!(page.state(), WifiPageState::Connected);
        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Enter),
        );
        assert_eq!(intent, WifiIntent::Disconnect);
    }

    #[test]
    fn auth_failure_returns_to_credential_entry_empty() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        for &b in b"passw0rd" {
            page.update(
                &snap(RadioState::Enabled, OperationState::Results, None),
                [result(1, Security::Wpa2PersonalAes)].into_iter(),
                Some(WifiKey::Char(b)),
            );
        }
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(page.state(), WifiPageState::Connecting);

        page.update(
            &snap(
                RadioState::Enabled,
                OperationState::Failed,
                Some(NetworkError::AuthenticationFailed),
            ),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            None,
        );
        assert_eq!(page.state(), WifiPageState::CredentialEntry);
        assert!(page.credential_zeroized());
    }

    #[test]
    fn saved_profile_connect_skips_secret_copy_into_page() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(7, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        assert_eq!(page.state(), WifiPageState::CredentialEntry);

        let intent = page.begin_saved_connect(7, Security::Wpa2PersonalAes);
        assert_eq!(
            intent,
            WifiIntent::Connect {
                result_id: 7,
                security: Security::Wpa2PersonalAes,
            }
        );
        assert_eq!(page.state(), WifiPageState::Connecting);
        assert!(page.credential_zeroized());
    }

    #[test]
    fn radio_loss_overrides_and_zeroizes() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Char(b'a')),
        );
        assert_eq!(page.state(), WifiPageState::CredentialEntry);

        page.update(
            &snap(
                RadioState::Unavailable,
                OperationState::RadioUnavailable,
                Some(NetworkError::RadioUnavailable),
            ),
            no_results(),
            None,
        );
        assert_eq!(page.state(), WifiPageState::RadioUnavailable);
        assert!(page.credential_zeroized());

        let intent = page.update(
            &snap(
                RadioState::Unavailable,
                OperationState::RadioUnavailable,
                Some(NetworkError::RadioUnavailable),
            ),
            no_results(),
            Some(WifiKey::Esc),
        );
        assert_eq!(intent, WifiIntent::Exit);
    }

    #[test]
    fn forget_confirm_flow() {
        let mut page = WifiPageController::new();
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            None,
        );
        page.target_result_id = 3;
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Char(b'f')),
        );
        assert_eq!(page.state(), WifiPageState::ForgetConfirm);
        assert!(!page.forget_choice_is_forget());
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Esc),
        );
        assert_eq!(page.state(), WifiPageState::Connected);
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Char(b'f')),
        );
        page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Right),
        );
        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Connected, None),
            no_results(),
            Some(WifiKey::Enter),
        );
        assert_eq!(intent, WifiIntent::Forget { profile_id: 3 });
    }

    #[test]
    fn scanning_esc_cancels() {
        let mut page = WifiPageController::new();
        let intent = page.update(
            &snap(RadioState::Enabled, OperationState::Scanning, None),
            no_results(),
            Some(WifiKey::Esc),
        );
        assert_eq!(intent, WifiIntent::Cancel);
    }

    #[test]
    fn bssid_field_is_accessible() {
        let r = result(5, Security::Open);
        assert_eq!(r.bssid.len(), BSSID_BYTES);
    }

    #[test]
    fn tab_navigation_matches_directional_order_and_reset_zeroizes() {
        let mut page = WifiPageController::new();
        let rows = [
            result(1, Security::Open),
            result(2, Security::Wpa2PersonalAes),
        ];
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            rows.into_iter(),
            Some(WifiKey::Next),
        );
        assert_eq!(page.selected(), 1);
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            rows.into_iter(),
            Some(WifiKey::Previous),
        );
        assert_eq!(page.selected(), 0);

        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Enter),
        );
        page.update(
            &snap(RadioState::Enabled, OperationState::Results, None),
            [result(1, Security::Wpa2PersonalAes)].into_iter(),
            Some(WifiKey::Char(b'x')),
        );
        page.reset();
        assert_eq!(page.state(), WifiPageState::Disabled);
        assert!(page.credential_zeroized());
        assert_eq!(page.row_count(), 0);
    }
}
