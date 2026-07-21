use koto_core::{
    BitmapFont, CanvasUiPainter, ConfigService, KotoConfigUi, KotoConfigWifiUi, Locale,
    NetworkError, NetworkSnapshot, OperationState, RadioState, ScanResult, Security, Ssid, WifiKey,
    WifiPageState, KOTOCONFIG_SURFACE, KOTOCONFIG_WIFI_SURFACE,
};
use koto_sim::Framebuffer;

const FONT: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");
const ENGLISH_GOLDEN: u64 = 0x3147702a1c9ebe29;
const JAPANESE_GOLDEN: u64 = 0xe96b4753db59c34b;
const WIFI_ENGLISH_GOLDENS: [u64; 9] = [
    0x60159265d011fddd,
    0x2a77f99eb3537de9,
    0x381845e914a0bccc,
    0x01057f4efd2d14bd,
    0x9ed7cfb07c6c0f29,
    0x20d9ca95ab0e752d,
    0xaaee2a0867318e8d,
    0xf4b9aaeecc9a56d1,
    0x33443299fa807b6a,
];
const WIFI_JAPANESE_GOLDENS: [u64; 9] = [
    0x19061c26997620cd,
    0x5c464ced80b7bb4d,
    0x57ad5e7d3ddadb24,
    0x85b5c21358ccd4a9,
    0x1e6ede4bec66cea1,
    0xf67ac1078f875dee,
    0xd784cdd96480706e,
    0x5dc6ec4bf5c89e92,
    0xe897cc4a428fb699,
];

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn render(locale: Locale) -> u64 {
    let font = BitmapFont::from_bytes(FONT).unwrap();
    let mut config = ConfigService::default();
    config.set_locale(locale);
    let ui = KotoConfigUi::new(&config);
    let mut framebuffer = Framebuffer::new(320, 320);
    let mut canvas = framebuffer.as_canvas();
    let mut painter = CanvasUiPainter::new(&mut canvas, &font);
    ui.paint(&mut painter, KOTOCONFIG_SURFACE).unwrap();
    hash(framebuffer.as_canvas().pixels())
}

#[test]
fn english_and_japanese_frames_match_goldens() {
    let english = render(Locale::EnUs);
    let japanese = render(Locale::JaJp);
    println!("KotoConfig english={english:#018x} japanese={japanese:#018x}");
    assert_ne!(english, japanese);
    assert_eq!(english, ENGLISH_GOLDEN);
    assert_eq!(japanese, JAPANESE_GOLDEN);
}

fn wifi_snapshot(state: OperationState, error: Option<NetworkError>) -> NetworkSnapshot {
    NetworkSnapshot {
        generation: 1,
        request_id: 1,
        radio: match state {
            OperationState::Disabled => RadioState::Disabled,
            OperationState::RadioUnavailable => RadioState::Unavailable,
            _ => RadioState::Enabled,
        },
        state,
        connected_result_id: (state == OperationState::Connected).then_some(1),
        result_count: 2,
        retry_count: 1,
        deadline_ms_remaining: 12_000,
        last_error: error,
        command_depth: 0,
        event_depth: 0,
    }
}

fn wifi_rows() -> [ScanResult; 2] {
    [
        ScanResult {
            result_id: 1,
            ssid: Ssid::from_bytes(b"KotoLab"),
            bssid: [2, 0, 0, 0, 0, 1],
            rssi_dbm: -42,
            security: Security::Wpa2PersonalAes,
        },
        ScanResult {
            result_id: 2,
            ssid: Ssid::from_bytes(&[0xff, b'K', b'o', b't', b'o']),
            bssid: [2, 0, 0, 0, 0, 2],
            rssi_dbm: -67,
            security: Security::Open,
        },
    ]
}

fn render_wifi(locale: Locale, state: WifiPageState) -> u64 {
    let font = BitmapFont::from_bytes(FONT).unwrap();
    let rows = wifi_rows();
    let (service_state, error) = match state {
        WifiPageState::Disabled => (OperationState::Disabled, None),
        WifiPageState::Scanning => (OperationState::Scanning, None),
        WifiPageState::Results | WifiPageState::CredentialEntry => (OperationState::Results, None),
        WifiPageState::Connecting => (OperationState::Connecting, None),
        WifiPageState::Connected | WifiPageState::ForgetConfirm => {
            (OperationState::Connected, None)
        }
        WifiPageState::Failed => (OperationState::Failed, Some(NetworkError::Timeout)),
        WifiPageState::RadioUnavailable => (
            OperationState::RadioUnavailable,
            Some(NetworkError::RadioUnavailable),
        ),
    };
    let snapshot = wifi_snapshot(service_state, error);
    let mut ui = KotoConfigWifiUi::new(locale, snapshot);
    ui.update(snapshot, rows.into_iter(), None);
    match state {
        WifiPageState::CredentialEntry => {
            ui.update(snapshot, rows.into_iter(), Some(WifiKey::Enter));
            for &byte in b"secret12" {
                ui.update(snapshot, rows.into_iter(), Some(WifiKey::Char(byte)));
            }
        }
        WifiPageState::ForgetConfirm => {
            ui.update(snapshot, rows.into_iter(), Some(WifiKey::Char(b'f')));
        }
        _ => {}
    }
    assert_eq!(ui.state(), state);
    let mut framebuffer = Framebuffer::new(320, 320);
    let mut canvas = framebuffer.as_canvas();
    let mut painter = CanvasUiPainter::new(&mut canvas, &font);
    ui.paint(&mut painter, KOTOCONFIG_WIFI_SURFACE).unwrap();
    hash(framebuffer.as_canvas().pixels())
}

#[test]
fn wifi_states_match_english_and_japanese_goldens() {
    let states = [
        WifiPageState::Disabled,
        WifiPageState::Scanning,
        WifiPageState::Results,
        WifiPageState::CredentialEntry,
        WifiPageState::Connecting,
        WifiPageState::Connected,
        WifiPageState::Failed,
        WifiPageState::ForgetConfirm,
        WifiPageState::RadioUnavailable,
    ];
    let english = states.map(|state| render_wifi(Locale::EnUs, state));
    let japanese = states.map(|state| render_wifi(Locale::JaJp, state));
    println!("Wi-Fi English: {english:#018x?}");
    println!("Wi-Fi Japanese: {japanese:#018x?}");
    assert_eq!(english, WIFI_ENGLISH_GOLDENS);
    assert_eq!(japanese, WIFI_JAPANESE_GOLDENS);
}
